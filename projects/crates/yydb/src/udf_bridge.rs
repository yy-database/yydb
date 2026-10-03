//! Bridge between the legacy scalar UDF API and `yydb-udf`.

use std::sync::Arc;

use yydb_execution::ValidatedUdf;
use yydb_types::{Error, Result, Value};
use yydb_udf::{
    Budget, NativeHandler, NativeUdfDefinition, Placement, RegisterOptions, Signature, UdfError,
    HostMicroDefinition, HostMicroImplementation, HostRuntimeAdapter, UdfIdentity,
    UdfInvocation, UdfPolicy, UdfRegistry, UdfType, UdfValue, VosProgramImplementation,
    lower_micro_scalar, lower_vos_macro,
};

type UdfBridgeResult<T> = std::result::Result<T, UdfError>;

use crate::udf::{validate_local_execution_body, RegisteredUdf, ScalarFn, ScalarUdf};

pub(crate) struct UdfSubsystem {
    registry: UdfRegistry,
    legacy: std::collections::BTreeMap<String, RegisteredUdf>,
    host_adapter: Option<Arc<dyn HostRuntimeAdapter>>,
}

impl Default for UdfSubsystem {
    fn default() -> Self {
        Self {
            registry: UdfRegistry::default(),
            legacy: std::collections::BTreeMap::new(),
            host_adapter: None,
        }
    }
}

impl UdfSubsystem {
    pub(crate) fn register_scalar_versioned(
        &mut self,
        name: &str,
        version: u32,
        udf: Arc<dyn ScalarUdf>,
    ) -> Result<()> {
        if name.is_empty() {
            return Err(Error::Udf {
                name: String::new(),
                message: "UDF name must not be empty".into(),
            });
        }
        self.legacy
            .insert(name.to_owned(), RegisteredUdf { udf, version });
        Ok(())
    }

    pub(crate) fn register_execution_udf(&mut self, body: ValidatedUdf) -> Result<()> {
        validate_local_execution_body(&body)?;
        let lowered = execution_body_to_lowered(body).map_err(map_udf_error)?;
        self.install_session_lowered(lowered, RegisterOptions::new())?;
        Ok(())
    }

    pub(crate) fn register_vos_scalar(&mut self, source: &str, version: u32) -> Result<()> {
        let lowered = lower_micro_scalar(source, version).map_err(map_udf_error)?;
        self.install_session_lowered(lowered, RegisterOptions::new())?;
        Ok(())
    }

    pub(crate) fn register_vos_macro(&mut self, source: &str, version: u32) -> Result<()> {
        let lowered = lower_vos_macro(source, version).map_err(map_udf_error)?;
        self.install_catalog_lowered(lowered)?;
        Ok(())
    }

    pub(crate) fn register_closure_scalar(
        &mut self,
        name: &str,
        version: u32,
        arity: Option<usize>,
        func: Arc<ScalarFn>,
    ) -> Result<()> {
        let definition = native_from_closure(name, version, arity, func).map_err(map_udf_error)?;
        self.register_native_udf(definition)
    }

    pub(crate) fn set_host_adapter(&mut self, adapter: Arc<dyn HostRuntimeAdapter>) {
        self.host_adapter = Some(adapter);
    }

    pub(crate) fn register_host_micro(&mut self, definition: HostMicroDefinition) -> Result<()> {
        definition.validate().map_err(map_udf_error)?;
        let session = definition.session_definition();
        let name = session.identity.name().to_owned();
        if let Some(adapter) = self.host_adapter.clone() {
            let implementation = Arc::new(HostMicroImplementation::new(definition, adapter));
            self.registry
                .register_session_micro(session, implementation, RegisterOptions::new())
                .map_err(map_udf_error)?;
        } else {
            self.registry
                .register_session(session, RegisterOptions::new())
                .map_err(map_udf_error)?;
        }
        self.legacy.remove(&name);
        Ok(())
    }

    pub(crate) fn register_native_udf(&mut self, definition: NativeUdfDefinition) -> Result<()> {
        definition.validate().map_err(map_udf_error)?;
        let session = definition.session_definition();
        let name = session.identity.name().to_owned();
        self.registry
            .register_session_micro(
                session,
                Arc::new(definition.into_implementation()),
                RegisterOptions::new(),
            )
            .map_err(map_udf_error)?;
        self.legacy.remove(&name);
        Ok(())
    }

    pub(crate) fn remove_scalar(&mut self, name: &str) -> Result<()> {
        if self.legacy.remove(name).is_some() {
            return Ok(());
        }
        self.registry
            .remove_session_by_name(name)
            .map_err(map_udf_error)
    }

    pub(crate) fn list_scalars(&self) -> Vec<String> {
        let mut names: std::collections::BTreeSet<String> = self.legacy.keys().cloned().collect();
        for name in self.registry.catalog_names() {
            names.insert(name);
        }
        for name in self.registry.session_names() {
            names.insert(name);
        }
        names.into_iter().collect()
    }

    pub(crate) fn call_scalar_version(
        &self,
        name: &str,
        version: u32,
        args: &[Value],
    ) -> Result<Value> {
        let identity = UdfIdentity::new("", name, version).map_err(map_udf_error)?;
        if let Some(definition) = self.registry.resolve_definition(&identity) {
            let expected = definition.signature.arity();
            if args.len() != expected {
                return Err(Error::UdfArity {
                    name: name.to_owned(),
                    expected,
                    got: args.len(),
                });
            }
            let udf_args = values_to_udf(args)?;
            let result = self
                .registry
                .invoke(
                    &UdfInvocation::scalar(identity, udf_args),
                    Budget::new(64),
                )
                .map_err(|error| map_udf_error_with_name(error, name, expected, args.len()))?;
            return udf_to_value(result);
        }

        if let Some(expected) = self.registry.version_for_name(name) {
            if expected != version {
                return Err(Error::UdfVersionMismatch {
                    name: name.to_owned(),
                    expected,
                    got: version,
                });
            }
        }

        let entry = self.legacy.get(name).ok_or_else(|| Error::UdfNotFound {
            name: name.to_owned(),
        })?;
        if entry.version != version {
            return Err(Error::UdfVersionMismatch {
                name: name.to_owned(),
                expected: entry.version,
                got: version,
            });
        }
        if let Some(expected) = entry.udf.arity() {
            if args.len() != expected {
                return Err(Error::UdfArity {
                    name: name.to_owned(),
                    expected,
                    got: args.len(),
                });
            }
        }
        entry.udf.call(args).map_err(|error| match error {
            Error::Udf { .. }
            | Error::UdfArity { .. }
            | Error::UdfNotFound { .. }
            | Error::UdfVersionMismatch { .. }
            | Error::ObjectNotFound { .. }
            | Error::ObjectCorrupt { .. }
            | Error::Unsupported(_) => error,
            other => Error::Udf {
                name: name.to_owned(),
                message: other.to_string(),
            },
        })
    }

    fn install_session_lowered(
        &mut self,
        lowered: yydb_udf::LoweredUdf,
        options: RegisterOptions,
    ) -> Result<()> {
        let name = lowered.identity.name().to_owned();
        let definition = lowered.session_definition();
        let implementation = Arc::new(VosProgramImplementation::new(lowered));
        self.registry
            .register_session(definition, options)
            .map_err(map_udf_error)?;
        self.registry
            .bind_host(implementation)
            .map_err(map_udf_error)?;
        self.legacy.remove(&name);
        Ok(())
    }

    fn install_catalog_lowered(&mut self, lowered: yydb_udf::LoweredUdf) -> Result<()> {
        let name = lowered.identity.name().to_owned();
        let definition = lowered.catalog_definition();
        let implementation = Arc::new(VosProgramImplementation::new(lowered));
        self.registry
            .register_catalog_macro(definition, implementation)
            .map_err(map_udf_error)?;
        self.legacy.remove(&name);
        Ok(())
    }
}

fn execution_body_to_lowered(body: ValidatedUdf) -> UdfBridgeResult<yydb_udf::LoweredUdf> {
    let mut arg_types = Vec::new();
    for ty in body.input_types() {
        arg_types.push(UdfType::from_execution(*ty)?);
    }
    let return_ty = UdfType::from_execution(body.output_type())?;
    let identity = UdfIdentity::new("", body.id(), body.version())?;
    Ok(yydb_udf::LoweredUdf {
        identity,
        signature: Signature::new(arg_types, return_ty),
        policy: UdfPolicy::pure_embedded(),
        placement: Placement::Embedded,
        program: body,
        source_fingerprint: [0u8; 32],
    })
}

fn values_to_udf(args: &[Value]) -> Result<Vec<UdfValue>> {
    let mut out = Vec::with_capacity(args.len());
    for value in args {
        out.push(match value {
            Value::Bool(value) => UdfValue::Bool(*value),
            Value::I64(value) => UdfValue::I64(*value),
            Value::Text(value) => UdfValue::Text(value.clone()),
            Value::Null | _ => {
                return Err(Error::Unsupported(
                    "value type is not supported by the yydb-udf scalar bridge",
                ))
            }
        });
    }
    Ok(out)
}

fn udf_to_value(value: UdfValue) -> Result<Value> {
    match value {
        UdfValue::Null => Ok(Value::Null),
        UdfValue::Bool(value) => Ok(Value::Bool(value)),
        UdfValue::I64(value) => Ok(Value::I64(value)),
        UdfValue::Text(value) => Ok(Value::Text(value)),
    }
}

fn map_udf_error(error: UdfError) -> Error {
    map_udf_error_with_name(error, "<udf>", 0, 0)
}

fn map_udf_error_with_name(
    error: UdfError,
    name: &str,
    expected_arity: usize,
    got_arity: usize,
) -> Error {
    match error {
        UdfError::NotFound => Error::UdfNotFound { name: name.to_owned() },
        UdfError::VersionMismatch => Error::UdfVersionMismatch {
            name: name.to_owned(),
            expected: 0,
            got: 0,
        },
        UdfError::ArityMismatch => Error::UdfArity {
            name: name.to_owned(),
            expected: expected_arity,
            got: got_arity,
        },
        UdfError::UnsupportedType | UdfError::UnsupportedEffect => Error::Unsupported(
            "value or effect is outside the supported yydb-udf subset",
        ),
        UdfError::SignatureMismatch => Error::Udf {
            name: name.to_owned(),
            message: "UDF argument types do not match the registered signature".into(),
        },
        other => Error::Udf {
            name: name.to_owned(),
            message: other.to_string(),
        },
    }
}

pub(crate) fn native_from_closure(
    name: &str,
    version: u32,
    arity: Option<usize>,
    func: Arc<ScalarFn>,
) -> UdfBridgeResult<NativeUdfDefinition> {
    let identity = UdfIdentity::new("", name, version)?;
    let signature = match arity {
        Some(1) => Signature::new(vec![UdfType::I64], UdfType::I64),
        Some(2) => Signature::new(vec![UdfType::I64, UdfType::I64], UdfType::I64),
        Some(_) => return Err(UdfError::InvalidDefinition),
        None => return Err(UdfError::InvalidDefinition),
    };
    let handler: NativeHandler = Arc::new(move |context, args| {
        let values = args
            .iter()
            .map(|value| match value {
                UdfValue::Null => Ok(Value::Null),
                UdfValue::Bool(value) => Ok(Value::Bool(*value)),
                UdfValue::I64(value) => Ok(Value::I64(*value)),
                UdfValue::Text(value) => Ok(Value::Text(value.clone())),
            })
            .collect::<UdfBridgeResult<Vec<_>>>()?;
        context.check_cancelled()?;
        let result = func(&values).map_err(|error| UdfError::ExecutionFailed {
            message: error.to_string(),
        })?;
        match result {
            Value::Null => Ok(UdfValue::Null),
            Value::Bool(value) => Ok(UdfValue::Bool(value)),
            Value::I64(value) => Ok(UdfValue::I64(value)),
            Value::Text(value) => Ok(UdfValue::Text(value)),
            _ => Err(UdfError::UnsupportedType),
        }
    });
    Ok(NativeUdfDefinition {
        identity,
        signature,
        policy: UdfPolicy::pure_embedded(),
        handler,
        fingerprint: [0u8; 32],
    })
}
