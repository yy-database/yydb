//! UDF surface for the embedded YYDB host.
//!
//! - **Simple path:** [`ScalarUdf`] + [`crate::Connection::create_scalar`].
//! - **Advanced path:** [`UdfRegistry`] and related types re-exported from
//!   `yydb-udf` below (`use yydb::udf::UdfRegistry`).
//!
//! Other languages should use `yydb-client` / `@yydb/yydb-client` against
//! `yydb serve` rather than embedding this crate.

use std::sync::Arc;

use yydb_execution::{UdfEffect, UdfPlacement, ValidatedUdf};
use yydb_types::{Result, Value};

pub(crate) struct ExecutionUdf {
    body: ValidatedUdf,
}

/// Validates a local read-only execution body before UDF registration.
pub(crate) fn validate_local_execution_body(body: &ValidatedUdf) -> Result<()> {
    if body.effect() != UdfEffect::Read || body.placement() != UdfPlacement::Local {
        return Err(yydb_types::Error::Unsupported(
            "local scalar execution requires a local read-only UDF",
        ));
    }
    if !body.parameter_types().is_empty() {
        return Err(yydb_types::Error::Unsupported(
            "local scalar execution does not bind captured parameters",
        ));
    }
    Ok(())
}

impl ExecutionUdf {
    pub(crate) fn new(body: ValidatedUdf) -> Result<Self> {
        validate_local_execution_body(&body)?;
        Ok(Self { body })
    }
}

impl ScalarUdf for ExecutionUdf {
    fn arity(&self) -> Option<usize> {
        Some(self.body.input_types().len())
    }

    fn call(&self, args: &[Value]) -> Result<Value> {
        let inputs = args
            .iter()
            .map(|value| match value {
                Value::Bool(value) => Ok(yydb_execution::Value::Bool(*value)),
                Value::I64(value) => Ok(yydb_execution::Value::I64(*value)),
                Value::Text(value) => Ok(yydb_execution::Value::Text(value.clone())),
                _ => Err(yydb_types::Error::Unsupported(
                    "value type is not supported by scalar execution",
                )),
            })
            .collect::<Result<Vec<_>>>()?;
        let result = self
            .body
            .evaluate(&[], &inputs)
            .map_err(|error| yydb_types::Error::Udf {
                name: self.body.id().to_owned(),
                message: format!("{error:?}"),
            })?;
        Ok(match result {
            yydb_execution::Value::Bool(value) => Value::Bool(value),
            yydb_execution::Value::I64(value) => Value::I64(value),
            yydb_execution::Value::Text(value) => Value::Text(value),
            _ => {
                return Err(yydb_types::Error::Unsupported(
                    "execution result type is not supported by the local scalar bridge",
                ))
            }
        })
    }
}

/// Object-safe scalar UDF invoked with [`Value`] arguments.
pub trait ScalarUdf: Send + Sync {
    /// Fixed arity, or `None` for variadic.
    fn arity(&self) -> Option<usize>;

    /// Execute the function.
    fn call(&self, args: &[Value]) -> Result<Value>;
}

/// Type alias for closure-style scalar UDFs.
pub type ScalarFn = dyn Fn(&[Value]) -> Result<Value> + Send + Sync;

pub(crate) struct ClosureUdf {
    arity: Option<usize>,
    func: Arc<ScalarFn>,
}

impl ClosureUdf {
    pub(crate) fn new(arity: Option<usize>, func: Arc<ScalarFn>) -> Self {
        Self { arity, func }
    }
}

impl ScalarUdf for ClosureUdf {
    fn arity(&self) -> Option<usize> {
        self.arity
    }

    fn call(&self, args: &[Value]) -> Result<Value> {
        (self.func)(args)
    }
}

pub(crate) struct RegisteredUdf {
    pub(crate) udf: Arc<dyn ScalarUdf>,
    pub(crate) version: u32,
}

/// Registry, host adapters, and contract types from `yydb-udf`.
pub use yydb_udf::{
    CatalogUdfEntry, CatalogUdfRegistry, HostFunctionHandle, HostMicroDefinition,
    HostMicroImplementation, HostRuntimeAdapter, HostUdfEntry, HostUdfRegistry, RegisterOptions,
    SessionUdfEntry, SessionUdfRegistry, Signature, UdfDefinition, UdfError, UdfIdentity,
    UdfRegistry, UdfType, UdfValue,
};
