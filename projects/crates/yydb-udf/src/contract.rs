//! UDF definition contracts shared by catalog, session, and host registries.

use crate::capability::{Determinism, Effect, Placement, UdfPolicy};
use crate::error::Result;
use crate::error::UdfError;
use crate::identity::UdfIdentity;

/// Phase-1 scalar types supported by the UDF boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UdfType {
    /// Explicit null.
    Null,
    /// Boolean.
    Bool,
    /// Signed 64-bit integer.
    I64,
    /// UTF-8 text.
    Text,
}

/// Argument and return signature for a UDF.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Signature {
    /// Positional argument types.
    pub args: Vec<UdfType>,
    /// Return type.
    pub returns: UdfType,
}

impl Signature {
    /// Creates a new signature.
    pub fn new(args: Vec<UdfType>, returns: UdfType) -> Self {
        Self { args, returns }
    }

    /// Returns the expected argument count.
    pub fn arity(&self) -> usize {
        self.args.len()
    }

    /// Verifies that `actual` matches this signature.
    pub fn ensure_args(&self, actual: &[UdfType]) -> Result<()> {
        if self.args != actual {
            return Err(UdfError::SignatureMismatch);
        }
        Ok(())
    }
}

/// Implementation kind recorded in catalog metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImplementationKind {
    /// Lowered `yydb-execution` program from VOS macro/micro source.
    VosProgram,
    /// Rust native handler in the embedded process.
    Native,
    /// TypeScript host callback.
    TypeScript,
}

/// Catalog-visible UDF metadata without host callbacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdfDefinition {
    /// Logical identity.
    pub identity: UdfIdentity,
    /// Typed signature.
    pub signature: Signature,
    /// Execution policy.
    pub policy: UdfPolicy,
    /// Placement requirement.
    pub placement: Placement,
    /// Implementation kind persisted or advertised by the catalog.
    pub implementation_kind: ImplementationKind,
    /// Source or implementation fingerprint.
    pub fingerprint: [u8; 32],
}

impl UdfDefinition {
    /// Validates a catalog definition.
    pub fn validate(&self) -> Result<()> {
        if self.policy.effect == Effect::Pure
            && self.policy.determinism != Determinism::Deterministic
        {
            return Err(UdfError::UnsupportedEffect);
        }
        if self.placement == Placement::Embedded
            && matches!(
                self.policy.effect,
                Effect::Network | Effect::WriteLocal | Effect::ExternalSideEffect
            )
        {
            return Err(UdfError::UnsupportedEffect);
        }
        Ok(())
    }
}
