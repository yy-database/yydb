//! YYDB UDF subsystem: contract, registry, and host adapters.
//!
//! This crate sits above [`yydb-execution`] and below the `yydb` facade. It does
//! not replace the execution model and is not shared with YYDS.

#![warn(missing_docs)]

mod capability;
mod contract;
mod error;
mod identity;
mod implementation;
mod invocation;
mod native;
mod registry;
mod typescript;
mod value;
mod vos;

pub use capability::{
    CapabilitySet, Determinism, Effect, Placement, UdfPolicy,
};
pub use contract::{
    ImplementationKind, Signature, UdfDefinition, UdfType,
};
pub use error::{Result, UdfError};
pub use identity::{UdfFingerprint, UdfIdentity};
pub use implementation::UdfImplementation;
pub use invocation::{Budget, InvocationMode, UdfContext, UdfInvocation};
pub use native::{NativeHandler, NativeImplementation, NativeUdfDefinition};
pub use registry::{
    CatalogUdfEntry, CatalogUdfRegistry, HostUdfEntry, HostUdfRegistry, RegisterOptions,
    SessionUdfEntry, SessionUdfRegistry, UdfRegistry,
};
pub use typescript::{
    TypeScriptFunctionHandle, TypeScriptHostAdapter, TypeScriptMicroDefinition,
    TypeScriptMicroImplementation,
};
pub use value::UdfValue;
pub use vos::{LoweredUdf, VosProgramImplementation, lower_micro_scalar, lower_vos_macro};
