//! Validated execution program products for YYDB UDF registration.
//!
//! These types wrap an already-validated [`ValidatedUdf`](yydb_execution::ValidatedUdf)
//! body with the UDF identity, signature, and policy metadata needed by the
//! registry. They carry no VOS source and perform no lowering.

use yydb_execution::ValidatedUdf;

use crate::runtime::{UdfContext, UdfImplementation, from_execution, into_execution};
use crate::{
    ImplementationKind, Placement, Result, Signature, UdfDefinition, UdfError, UdfIdentity,
    UdfPolicy, UdfValue,
};

/// YYDB-internal product wrapping a validated execution program for registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionUdfBinding {
    /// Logical identity assigned during registration.
    pub identity: UdfIdentity,
    /// Boundary signature derived from the program.
    pub signature: Signature,
    /// Registration policy.
    pub policy: UdfPolicy,
    /// Placement requirement.
    pub placement: Placement,
    /// Validated local execution body.
    pub program: ValidatedUdf,
    /// Fingerprint of the original authored source, when known.
    pub source_fingerprint: [u8; 32],
}

impl ExecutionUdfBinding {
    /// Returns the execution identity string stored in `yydb-execution`.
    pub fn execution_id(&self) -> &str {
        self.program.id()
    }

    /// Builds session metadata for a registered execution program.
    pub fn session_definition(&self) -> UdfDefinition {
        UdfDefinition {
            identity: self.identity.clone(),
            signature: self.signature.clone(),
            policy: self.policy,
            placement: self.placement,
            implementation_kind: ImplementationKind::ExecutionProgram,
            fingerprint: self.source_fingerprint,
        }
    }
}

/// Embedded execution program implementation strategy.
pub struct ExecutionProgramImplementation {
    binding: ExecutionUdfBinding,
}

impl ExecutionProgramImplementation {
    /// Creates an implementation from a validated execution program binding.
    pub fn new(binding: ExecutionUdfBinding) -> Self {
        Self { binding }
    }
}

impl UdfImplementation for ExecutionProgramImplementation {
    fn identity(&self) -> &UdfIdentity {
        &self.binding.identity
    }

    fn signature(&self) -> &Signature {
        &self.binding.signature
    }

    fn invoke(&self, context: &mut UdfContext, args: &[UdfValue]) -> Result<UdfValue> {
        self.binding
            .signature
            .ensure_args(&UdfValue::argument_types(args))?;
        context.check_cancelled()?;
        context.charge(1)?;
        let execution_args = args
            .iter()
            .cloned()
            .map(into_execution)
            .collect::<Result<Vec<_>>>()?;
        let result = self
            .binding
            .program
            .evaluate(&[], &execution_args)
            .map_err(|error| UdfError::ExecutionFailed {
                message: format!("{error:?}"),
            })?;
        from_execution(&result)
    }
}
