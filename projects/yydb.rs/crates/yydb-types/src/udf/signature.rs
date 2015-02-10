//! UDF definition contracts shared by catalog, session, and host registries.

use super::{
    Result, UdfError,
    identity::UdfIdentity,
    policy::{Determinism, Effect, Placement, UdfPolicy},
};

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
    /// Accepts any Phase-1 argument type. Used only for embedded closure contracts.
    Any,
}

/// Argument and return signature for a UDF.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Signature {
    /// Positional argument types.
    pub args: Vec<UdfType>,
    /// Return type.
    pub returns: UdfType,
    /// When true, only the Phase-1 value subset is checked and arity is not fixed.
    pub variadic: bool,
}

impl Signature {
    /// Creates a fully typed signature.
    pub fn new(args: Vec<UdfType>, returns: UdfType) -> Self {
        Self { args, returns, variadic: false }
    }

    /// Creates a fixed-arity closure contract that accepts any Phase-1 argument types.
    pub fn untyped_fixed(arity: usize) -> Self {
        Self { args: vec![UdfType::Any; arity], returns: UdfType::Any, variadic: false }
    }

    /// Creates a variadic closure contract that accepts any Phase-1 argument count and types.
    pub fn untyped_variadic() -> Self {
        Self { args: vec![], returns: UdfType::Any, variadic: true }
    }

    /// Returns the fixed argument count, if any.
    pub fn expected_arity(&self) -> Option<usize> {
        if self.variadic { None } else { Some(self.args.len()) }
    }

    /// Returns the expected argument count for typed signatures.
    pub fn arity(&self) -> usize {
        self.args.len()
    }

    /// Verifies that `actual` matches this signature.
    pub fn ensure_args(&self, actual: &[UdfType]) -> Result<()> {
        if self.variadic {
            return Ok(());
        }
        if self.args.len() != actual.len() {
            return Err(UdfError::ArityMismatch);
        }
        for (expected, actual) in self.args.iter().zip(actual) {
            if *expected != UdfType::Any && *expected != *actual {
                return Err(UdfError::SignatureMismatch);
            }
        }
        Ok(())
    }
}

/// Implementation kind recorded in catalog metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImplementationKind {
    /// Validated `yydb-execution` program bound at registration time.
    ExecutionProgram,
    /// Rust native handler in the embedded process.
    Native,
    /// Session micro implemented by an external host runtime.
    HostMicro,
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
        if self.policy.effect == Effect::Pure && self.policy.determinism != Determinism::Deterministic {
            return Err(UdfError::UnsupportedEffect);
        }
        if self.placement == Placement::Embedded
            && matches!(self.policy.effect, Effect::Network | Effect::WriteLocal | Effect::ExternalSideEffect)
        {
            return Err(UdfError::UnsupportedEffect);
        }
        Ok(())
    }
}
