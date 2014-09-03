//! Host runtime adapter and embedded host implementation strategy.

use crate::runtime::{InvocationMode, UdfContext, UdfImplementation};
use crate::{
    HostFunctionHandle, HostMicroDefinition, Result, Signature, UdfIdentity, UdfValue,
};

/// Adapter that owns the real function registry in an external host runtime.
pub trait HostRuntimeAdapter: Send + Sync {
    /// Invokes one scalar call through the host runtime.
    fn invoke_scalar(&self, handle: &HostFunctionHandle, args: &[UdfValue]) -> Result<UdfValue>;

    /// Invokes a bounded batch call through the host runtime.
    fn invoke_batch(
        &self,
        handle: &HostFunctionHandle,
        batches: &[Vec<UdfValue>],
    ) -> Result<Vec<UdfValue>>;
}

/// Host-side implementation strategy backed by a runtime adapter.
pub struct HostMicroImplementation {
    definition: HostMicroDefinition,
    adapter: std::sync::Arc<dyn HostRuntimeAdapter>,
}

impl HostMicroImplementation {
    /// Creates a host implementation from metadata and an adapter.
    pub fn new(
        definition: HostMicroDefinition,
        adapter: std::sync::Arc<dyn HostRuntimeAdapter>,
    ) -> Self {
        Self {
            definition,
            adapter,
        }
    }
}

impl UdfImplementation for HostMicroImplementation {
    fn identity(&self) -> &UdfIdentity {
        &self.definition.identity
    }

    fn signature(&self) -> &Signature {
        &self.definition.signature
    }

    fn invoke(&self, context: &mut UdfContext, args: &[UdfValue]) -> Result<UdfValue> {
        self.definition
            .signature
            .ensure_args(&UdfValue::argument_types(args))?;
        context.check_cancelled()?;
        context.charge(1)?;
        self.adapter.invoke_scalar(&self.definition.handle, args)
    }

    fn invoke_batch(
        &self,
        context: &mut UdfContext,
        batches: &[Vec<UdfValue>],
    ) -> Result<Vec<UdfValue>> {
        for batch in batches {
            self.definition
                .signature
                .ensure_args(&UdfValue::argument_types(batch))?;
        }
        context.check_cancelled()?;
        context.charge(batches.len() as u64)?;
        self.adapter.invoke_batch(&self.definition.handle, batches)
    }

    fn preferred_mode(&self) -> InvocationMode {
        InvocationMode::Batch
    }
}
