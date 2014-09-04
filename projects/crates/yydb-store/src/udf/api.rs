//! UDF surface for the embedded YYDB host.
//!
//! - **Simple path:** [`ScalarUdf`] + [`crate::Connection::create_scalar`].
//! - **Advanced path:** [`UdfRegistry`] and related types re-exported from
//!   `yydb-udf` below (`use yydb::udf::UdfRegistry`).
//!
//! Other languages should use `yydb-client` / `@yydb/yydb-client` against
//! `yydb serve` rather than embedding this crate.

use yydb_execution::{UdfEffect, UdfPlacement, ValidatedUdf};
use yydb_types::{Result, Value};

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

/// Object-safe scalar UDF invoked with [`Value`] arguments.
pub trait ScalarUdf: Send + Sync {
    /// Fixed arity, or `None` for variadic.
    fn arity(&self) -> Option<usize>;

    /// Execute the function.
    fn call(&self, args: &[Value]) -> Result<Value>;
}

/// Type alias for closure-style scalar UDFs.
pub type ScalarFn = dyn Fn(&[Value]) -> Result<Value> + Send + Sync;

/// Registry, host adapters, and contract types from `yydb-udf`.
pub use yydb_udf::{
    CatalogUdfEntry, CatalogUdfRegistry, HostFunctionHandle, HostMicroDefinition,
    HostMicroImplementation, HostRuntimeAdapter, ImplementationEntry, ImplementationRegistry,
    RegisterOptions,
    SessionUdfEntry, SessionUdfRegistry, Signature, UdfDefinition, UdfError, UdfIdentity,
    UdfRegistry, UdfType, UdfValue,
};
