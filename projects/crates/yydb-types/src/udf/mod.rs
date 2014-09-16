//! Pure UDF contracts shared by protocol, client, and runtime crates.
//!
//! Registry, dispatch, and evaluator adapters live in the `yydb-udf` crate.
//! These modules never depend on `yydb-execution`, the engine, or the wire
//! protocol.

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
pub use signature::{ImplementationKind, Signature, UdfDefinition, UdfType};
pub use value::UdfValue;
