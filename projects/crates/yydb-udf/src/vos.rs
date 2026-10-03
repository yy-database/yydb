//! VOS macro and micro lowering products for YYDB UDF registration.

mod lower;

use blake3::hash;
use yydb_execution::ValidatedUdf;

use crate::capability::Placement;
use crate::capability::UdfPolicy;
use crate::contract::{ImplementationKind, Signature, UdfDefinition};
use crate::error::{Result, UdfError};
use crate::identity::UdfIdentity;
use crate::implementation::UdfImplementation;
use crate::invocation::UdfContext;
use crate::value::UdfValue;

pub use lower::{lower_micro_scalar, lower_vos_macro};

/// YYDB-internal product of lowering a VOS macro or session micro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoweredUdf {
    /// Logical identity assigned during lowering.
    pub identity: UdfIdentity,
    /// Boundary signature derived from the lowered program.
    pub signature: Signature,
    /// Registration policy.
    pub policy: UdfPolicy,
    /// Placement requirement.
    pub placement: Placement,
    /// Validated local execution body.
    pub program: ValidatedUdf,
    /// Fingerprint of the original VOS source.
    pub source_fingerprint: [u8; 32],
}

impl LoweredUdf {
    /// Fingerprints VOS source for catalog metadata and cache keys.
    pub fn fingerprint_source(source: &str) -> [u8; 32] {
        hash(source.as_bytes()).into()
    }

    /// Returns the execution identity string stored in `yydb-execution`.
    pub fn execution_id(&self) -> &str {
        self.program.id()
    }

    /// Builds session metadata for a lowered VOS `micro`.
    pub fn session_definition(&self) -> UdfDefinition {
        UdfDefinition {
            identity: self.identity.clone(),
            signature: self.signature.clone(),
            policy: self.policy,
            placement: self.placement,
            implementation_kind: ImplementationKind::VosProgram,
            fingerprint: self.source_fingerprint,
        }
    }

    /// Builds catalog metadata for a lowered VOS `macro`.
    pub fn catalog_definition(&self) -> UdfDefinition {
        self.session_definition()
    }
}

/// Embedded VOS program implementation strategy.
pub struct VosProgramImplementation {
    lowered: LoweredUdf,
}

impl VosProgramImplementation {
    /// Creates an implementation from a lowered VOS UDF.
    pub fn new(lowered: LoweredUdf) -> Self {
        Self { lowered }
    }
}

impl UdfImplementation for VosProgramImplementation {
    fn identity(&self) -> &UdfIdentity {
        &self.lowered.identity
    }

    fn signature(&self) -> &Signature {
        &self.lowered.signature
    }

    fn invoke(&self, context: &mut UdfContext, args: &[UdfValue]) -> Result<UdfValue> {
        self.lowered
            .signature
            .ensure_args(&UdfValue::argument_types(args))?;
        context.check_cancelled()?;
        context.charge(1)?;
        let execution_args = args
            .iter()
            .cloned()
            .map(UdfValue::into_execution)
            .collect::<Result<Vec<_>>>()?;
        let result = self
            .lowered
            .program
            .evaluate(&[], &execution_args)
            .map_err(|error| UdfError::ExecutionFailed {
                message: format!("{error:?}"),
            })?;
        UdfValue::from_execution(&result)
    }
}
