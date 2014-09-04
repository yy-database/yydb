//! UDF session bridge: maps the embedded `Connection` API onto `yydb-udf`.

use std::sync::Arc;

use yydb_execution::ValidatedUdf;
use yydb_types::{Error, Result, Value};
use yydb_udf::{
    Budget, ExecutionProgramImplementation, HostMicroDefinition, HostMicroImplementation,
    HostRuntimeAdapter, NativeHandler, NativeUdfDefinition, Placement, RegisterOptions, Signature,
    UdfError, UdfIdentity, UdfInvocation, UdfPolicy, UdfRegistry, UdfValue,
};

type UdfBridgeResult<T> = std::result::Result<T, UdfError>;

use super::api::{validate_local_execution_body, ScalarFn, ScalarUdf};

#[derive(Default)]
pub(crate) struct UdfSubsystem {
    registry: UdfRegistry,
    host_adapter: Option<Arc<dyn HostRuntimeAdapter>>,
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
        let definition = native_from_scalar(name, version, udf).map_err(map_udf_error)?;
        self.register_native_udf(definition)
    }

    pub(crate) fn register_execution_udf(&mut self, body: ValidatedUdf) -> Result<()> {
        validate_local_execution_body(&body)?;
        let binding = execution_body_to_binding(body).map_err(map_udf_error)?;
        self.install_session_binding(binding, RegisterOptions::new())?;
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
        Ok(())
    }

    pub(crate) fn register_native_udf(&mut self, definition: NativeUdfDefinition) -> Result<()> {
        definition.validate().map_err(map_udf_error)?;
        let session = definition.session_definition();
        self.registry
            .register_session_micro(
                session,
                Arc::new(definition.into_implementation()),
                RegisterOptions::new(),
            )
            .map_err(map_udf_error)?;
        Ok(())
    }

    pub(crate) fn remove_scalar(&mut self, name: &str) -> Result<()> {
        self.registry
            .remove_session_by_name(name)
            .map_err(map_udf_error)
    }

    pub(crate) fn list_scalars(&self) -> Vec<String> {
        let mut names: std::collections::BTreeSet<String> = self.registry.catalog_names().into_iter().collect();
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
            if let Some(expected) = definition.signature.expected_arity() {
                if args.len() != expected {
                    return Err(Error::UdfArity {
                        name: name.to_owned(),
                        expected,
                        got: args.len(),
                    });
                }
            }
            let udf_args = values_to_udf(args)?;
            let result = self
                .registry
                .invoke(&UdfInvocation::scalar(identity, udf_args), Budget::new(64))
                .map_err(|error| map_udf_error_with_name(error, name, args.len()))?;
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

        Err(Error::UdfNotFound {
            name: name.to_owned(),
        })
    }

    fn install_session_binding(
        &mut self,
        binding: yydb_udf::ExecutionUdfBinding,
        options: RegisterOptions,
    ) -> Result<()> {
        let definition = binding.session_definition();
        let implementation = Arc::new(ExecutionProgramImplementation::new(binding));
        self.registry
            .register_session(definition, options)
            .map_err(map_udf_error)?;
        self.registry
            .bind_implementation(implementation)
            .map_err(map_udf_error)?;
        Ok(())
    }
}

fn execution_body_to_binding(body: ValidatedUdf) -> UdfBridgeResult<yydb_udf::ExecutionUdfBinding> {
    let mut arg_types = Vec::new();
    for ty in body.input_types() {
        arg_types.push(yydb_udf::type_from_execution(*ty)?);
    }
    let return_ty = yydb_udf::type_from_execution(body.output_type())?;
    let identity = UdfIdentity::new("", body.id(), body.version())?;
    Ok(yydb_udf::ExecutionUdfBinding {
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
            Value::Null => UdfValue::Null,
            Value::Bool(value) => UdfValue::Bool(*value),
            Value::I64(value) => UdfValue::I64(*value),
            Value::Text(value) => UdfValue::Text(value.clone()),
            _ => {
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
    map_udf_error_with_name(error, "<udf>", 0)
}

fn map_udf_error_with_name(error: UdfError, name: &str, got_arity: usize) -> Error {
    match error {
        UdfError::NotFound => Error::UdfNotFound {
            name: name.to_owned(),
        },
        UdfError::VersionMismatch => Error::UdfVersionMismatch {
            name: name.to_owned(),
            expected: 0,
            got: 0,
        },
        UdfError::ArityMismatch => Error::UdfArity {
            name: name.to_owned(),
            expected: 0,
            got: got_arity,
        },
        UdfError::UnsupportedType | UdfError::UnsupportedEffect => {
            Error::Unsupported("value or effect is outside the supported yydb-udf subset")
        }
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

fn native_from_scalar(
    name: &str,
    version: u32,
    udf: Arc<dyn ScalarUdf>,
) -> UdfBridgeResult<NativeUdfDefinition> {
    let identity = UdfIdentity::new("", name, version)?;
    let signature = match udf.arity() {
        Some(arity) => Signature::untyped_fixed(arity),
        None => Signature::untyped_variadic(),
    };
    let handler: NativeHandler = Arc::new(move |context, args| {
        let values = udf_values_to_store(args)?;
        context.check_cancelled()?;
        let result = udf.call(&values).map_err(|error| UdfError::ExecutionFailed {
            message: error.to_string(),
        })?;
        store_value_to_udf(result)
    });
    Ok(NativeUdfDefinition {
        identity,
        signature,
        policy: UdfPolicy::pure_embedded(),
        handler,
        fingerprint: [0u8; 32],
    })
}

pub(crate) fn native_from_closure(
    name: &str,
    version: u32,
    arity: Option<usize>,
    func: Arc<ScalarFn>,
) -> UdfBridgeResult<NativeUdfDefinition> {
    let identity = UdfIdentity::new("", name, version)?;
    let signature = match arity {
        Some(arity) => Signature::untyped_fixed(arity),
        None => Signature::untyped_variadic(),
    };
    let handler: NativeHandler = Arc::new(move |context, args| {
        let values = udf_values_to_store(args)?;
        context.check_cancelled()?;
        let result = func(&values).map_err(|error| UdfError::ExecutionFailed {
            message: error.to_string(),
        })?;
        store_value_to_udf(result)
    });
    Ok(NativeUdfDefinition {
        identity,
        signature,
        policy: UdfPolicy::pure_embedded(),
        handler,
        fingerprint: [0u8; 32],
    })
}

fn udf_values_to_store(args: &[UdfValue]) -> UdfBridgeResult<Vec<Value>> {
    args.iter()
        .map(|value| match value {
            UdfValue::Null => Ok(Value::Null),
            UdfValue::Bool(value) => Ok(Value::Bool(*value)),
            UdfValue::I64(value) => Ok(Value::I64(*value)),
            UdfValue::Text(value) => Ok(Value::Text(value.clone())),
        })
        .collect()
}

fn store_value_to_udf(value: Value) -> UdfBridgeResult<UdfValue> {
    match value {
        Value::Null => Ok(UdfValue::Null),
        Value::Bool(value) => Ok(UdfValue::Bool(value)),
        Value::I64(value) => Ok(UdfValue::I64(value)),
        Value::Text(value) => Ok(UdfValue::Text(value)),
        _ => Err(UdfError::UnsupportedType),
    }
}
