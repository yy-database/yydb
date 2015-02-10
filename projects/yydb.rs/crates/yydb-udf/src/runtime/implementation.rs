//! Strategy implementations invoked by the registry dispatcher.

use super::invocation::{InvocationMode, UdfContext};
use crate::{Result, Signature, UdfError, UdfIdentity, UdfValue};

/// One registered UDF implementation strategy.
pub trait UdfImplementation: Send + Sync {
    /// Returns the logical identity served by this implementation.
    fn identity(&self) -> &UdfIdentity;

    /// Returns the invocation signature.
    fn signature(&self) -> &Signature;

    /// Invokes the UDF with scalar arguments.
    fn invoke(&self, context: &mut UdfContext, args: &[UdfValue]) -> Result<UdfValue>;

    /// Invokes the UDF with a bounded batch when supported.
    fn invoke_batch(&self, context: &mut UdfContext, batches: &[Vec<UdfValue>]) -> Result<Vec<UdfValue>> {
        let _ = context;
        let _ = batches;
        Err(UdfError::UnsupportedEffect)
    }

    /// Returns the preferred invocation mode for planners.
    fn preferred_mode(&self) -> InvocationMode {
        InvocationMode::Scalar
    }
}
