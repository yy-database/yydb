//! YYDB UDF subsystem: registry, dispatch, and host adapters.
//!
//! Pure contracts live in [`yydb_udf_types`]. This crate sits above
//! [`yydb-execution`] and below the [`yydb`] facade. Applications should use
//! `yydb::udf` (`UdfRegistry`, `HostMicroDefinition`, …) instead of depending
//! here directly. Not shared with YYDS.

#![deny(missing_docs)]

mod host;
mod implementation;
mod invocation;
mod native;
mod program;
mod registry;
mod value_execution;

pub use yydb_udf_types::{
    CapabilitySet, Determinism, Effect, HostFunctionHandle, HostMicroDefinition,
    ImplementationKind, Placement, Result, Signature, UdfDefinition, UdfError, UdfFingerprint,
    UdfIdentity, UdfPolicy, UdfType, UdfValue,
};

pub use host::{HostMicroImplementation, HostRuntimeAdapter};
pub use implementation::UdfImplementation;
pub use invocation::{Budget, InvocationMode, UdfContext, UdfInvocation};
pub use native::{NativeHandler, NativeImplementation, NativeUdfDefinition};
pub use program::{LoweredUdf, VosProgramImplementation};
pub use registry::{
    CatalogUdfEntry, CatalogUdfRegistry, HostUdfEntry, HostUdfRegistry, RegisterOptions,
    SessionUdfEntry, SessionUdfRegistry, UdfRegistry,
};
pub use value_execution::{
    from_execution, into_execution, type_from_execution, type_to_execution,
};
