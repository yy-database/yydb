//! Rust native UDF definitions and embedded handlers.

use std::sync::Arc;

use crate::capability::{Placement, UdfPolicy};
use crate::contract::{ImplementationKind, Signature, UdfDefinition};
use crate::error::{Result, UdfError};
use crate::identity::UdfIdentity;
use crate::implementation::UdfImplementation;
use crate::invocation::UdfContext;
use crate::value::UdfValue;

/// Native handler invoked through the UDF boundary.
pub type NativeHandler =
    Arc<dyn Fn(&mut UdfContext, &[UdfValue]) -> Result<UdfValue> + Send + Sync>;

/// Registration contract for a Rust native UDF.
#[derive(Clone)]
pub struct NativeUdfDefinition {
    /// Logical identity.
    pub identity: UdfIdentity,
    /// Typed signature.
    pub signature: Signature,
    /// Execution policy.
    pub policy: UdfPolicy,
    /// Process-local handler.
    pub handler: NativeHandler,
    /// Implementation fingerprint for reopen checks.
    pub fingerprint: [u8; 32],
}

impl NativeUdfDefinition {
    /// Builds session metadata for this embedded native micro.
    pub fn session_definition(&self) -> UdfDefinition {
        UdfDefinition {
            identity: self.identity.clone(),
            signature: self.signature.clone(),
            policy: self.policy,
            placement: Placement::Embedded,
            implementation_kind: ImplementationKind::Native,
            fingerprint: self.fingerprint,
        }
    }

    /// Validates the definition before registration.
    pub fn validate(&self) -> Result<()> {
        self.session_definition().validate()?;
        if self.policy.effect != crate::capability::Effect::Pure {
            return Err(UdfError::UnsupportedEffect);
        }
        Ok(())
    }

    /// Wraps the handler as a [`UdfImplementation`].
    pub fn into_implementation(self) -> NativeImplementation {
        NativeImplementation {
            definition: self,
        }
    }
}

/// Embedded Rust implementation strategy.
#[derive(Clone)]
pub struct NativeImplementation {
    definition: NativeUdfDefinition,
}

impl UdfImplementation for NativeImplementation {
    fn identity(&self) -> &UdfIdentity {
        &self.definition.identity
    }

    fn signature(&self) -> &Signature {
        &self.definition.signature
    }

    fn invoke(&self, context: &mut UdfContext, args: &[UdfValue]) -> Result<UdfValue> {
        self.definition.signature.ensure_args(&UdfValue::argument_types(args))?;
        context.check_cancelled()?;
        context.charge(1)?;
        (self.definition.handler)(context, args)
    }
}
