//! Pure UDF contracts shared by protocol, client, and runtime crates.
//!
//! Registry, dispatch, and evaluator adapters live in [`yydb-udf`]. This crate
//! never depends on `yydb-execution` or the embedded engine.

#![deny(missing_docs)]

mod error;
mod host;
mod identity;
mod policy;
mod signature;
mod value;

pub use error::{Result, UdfError};
pub use host::{HostFunctionHandle, HostMicroDefinition};
pub use identity::{UdfFingerprint, UdfIdentity};
pub use policy::{CapabilitySet, Determinism, Effect, Placement, UdfPolicy};
pub use signature::{
    ImplementationKind, Signature, UdfDefinition, UdfType,
};
pub use value::UdfValue;
