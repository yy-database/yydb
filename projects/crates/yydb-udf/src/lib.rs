//! YYDB UDF subsystem: registry, dispatch, and host adapters.
//!
//! Pure contracts live in [`yydb_types::udf`]. This crate sits above
//! [`yydb-execution`] and below the [`yydb`] facade. Applications should use
//! `yydb::udf` (`UdfRegistry`, `HostMicroDefinition`, …) instead of depending
//! here directly. Not shared with YYDS.

#![deny(missing_docs)]

mod adapters;
mod registry;
mod runtime;

pub use yydb_types::udf::{
    CapabilitySet, Determinism, Effect, HostFunctionHandle, HostMicroDefinition, ImplementationKind, Placement, Result,
    Signature, UdfDefinition, UdfError, UdfFingerprint, UdfIdentity, UdfPolicy, UdfType, UdfValue,
};

pub use adapters::{
    ExecutionProgramImplementation, ExecutionUdfBinding, HostMicroImplementation, HostRuntimeAdapter, NativeHandler,
    NativeImplementation, NativeUdfDefinition,
};
pub use registry::{
    CatalogUdfEntry, CatalogUdfRegistry, ImplementationEntry, ImplementationRegistry, RegisterOptions, SessionUdfEntry,
    SessionUdfRegistry, UdfRegistry,
};
pub use runtime::{
    Budget, InvocationMode, UdfContext, UdfImplementation, UdfInvocation, from_execution, into_execution, type_from_execution,
    type_to_execution,
};
