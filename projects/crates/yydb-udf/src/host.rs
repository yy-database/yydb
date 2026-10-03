//! Session host micro handles and language-neutral host runtime adapter boundary.

use crate::capability::{Placement, UdfPolicy};
use crate::contract::{ImplementationKind, Signature, UdfDefinition};
use crate::error::{Result, UdfError};
use crate::identity::UdfIdentity;
use crate::implementation::UdfImplementation;
use crate::invocation::{InvocationMode, UdfContext};
use crate::value::UdfValue;

/// Opaque handle to a function registered in an external host runtime.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HostFunctionHandle {
    /// Host runtime instance identifier.
    pub host_id: u64,
    /// Function identifier inside the host registry.
    pub function_id: String,
    /// Monotonic implementation version.
    pub version: u32,
}

/// Session-local host micro metadata without persisting the host callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostMicroDefinition {
    /// Logical identity.
    pub identity: UdfIdentity,
    /// Typed signature.
    pub signature: Signature,
    /// Execution policy.
    pub policy: UdfPolicy,
    /// Opaque host handle.
    pub handle: HostFunctionHandle,
    /// Registration fingerprint.
    pub fingerprint: [u8; 32],
}

impl HostMicroDefinition {
    /// Builds a Phase-1 pure deterministic scalar micro from contract fields.
    pub fn from_scalar(
        name: &str,
        version: u32,
        args: Vec<crate::contract::UdfType>,
        returns: crate::contract::UdfType,
        host_id: u64,
        function_id: impl Into<String>,
        handle_version: u32,
        fingerprint: [u8; 32],
    ) -> Result<Self> {
        let identity = UdfIdentity::new("", name, version)?;
        let signature = Signature::new(args, returns);
        let function_id = function_id.into();
        Ok(Self {
            identity,
            signature,
            policy: UdfPolicy::pure_host(),
            handle: HostFunctionHandle {
                host_id,
                function_id,
                version: handle_version,
            },
            fingerprint,
        })
    }

    /// Builds session metadata for this host micro.
    ///
    /// Host micros never enter the `.yydb` catalog.
    pub fn session_definition(&self) -> UdfDefinition {
        UdfDefinition {
            identity: self.identity.clone(),
            signature: self.signature.clone(),
            policy: self.policy,
            placement: Placement::Host,
            implementation_kind: ImplementationKind::HostMicro,
            fingerprint: self.fingerprint,
        }
    }

    /// Validates metadata before host installation.
    pub fn validate(&self) -> Result<()> {
        self.session_definition().validate()?;
        if self.handle.function_id.is_empty() {
            return Err(UdfError::InvalidDefinition);
        }
        if self.handle.version == 0 {
            return Err(UdfError::VersionMismatch);
        }
        Ok(())
    }
}

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
