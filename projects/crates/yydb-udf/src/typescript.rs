//! TypeScript session micro handles and host adapter boundary.

use crate::capability::{Placement, UdfPolicy};
use crate::contract::{ImplementationKind, Signature, UdfDefinition};
use crate::error::{Result, UdfError};
use crate::identity::UdfIdentity;
use crate::implementation::UdfImplementation;
use crate::invocation::{InvocationMode, UdfContext};
use crate::value::UdfValue;

/// Opaque handle to a process-local TypeScript function.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeScriptFunctionHandle {
    /// Host runtime instance identifier.
    pub host_id: u64,
    /// Function identifier inside the TS registry.
    pub function_id: String,
    /// Monotonic implementation version.
    pub version: u32,
}

/// Session-local TypeScript micro metadata without persisting the JS closure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeScriptMicroDefinition {
    /// Logical identity.
    pub identity: UdfIdentity,
    /// Typed signature.
    pub signature: Signature,
    /// Execution policy.
    pub policy: UdfPolicy,
    /// Opaque host handle.
    pub handle: TypeScriptFunctionHandle,
    /// Registration fingerprint.
    pub fingerprint: [u8; 32],
}

impl TypeScriptMicroDefinition {
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
            handle: TypeScriptFunctionHandle {
                host_id,
                function_id,
                version: handle_version,
            },
            fingerprint,
        })
    }

    /// Builds session metadata for this host micro.
    ///
    /// TypeScript micros never enter the `.yydb` catalog.
    pub fn session_definition(&self) -> UdfDefinition {
        UdfDefinition {
            identity: self.identity.clone(),
            signature: self.signature.clone(),
            policy: self.policy,
            placement: Placement::Host,
            implementation_kind: ImplementationKind::TypeScriptMicro,
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

/// Host adapter that owns the real JS function registry.
pub trait TypeScriptHostAdapter: Send + Sync {
    /// Invokes one scalar call through the host runtime.
    fn invoke_scalar(
        &self,
        handle: &TypeScriptFunctionHandle,
        args: &[UdfValue],
    ) -> Result<UdfValue>;

    /// Invokes a bounded batch call through the host runtime.
    fn invoke_batch(
        &self,
        handle: &TypeScriptFunctionHandle,
        batches: &[Vec<UdfValue>],
    ) -> Result<Vec<UdfValue>>;
}

/// Host-side implementation strategy backed by a TS adapter.
pub struct TypeScriptMicroImplementation {
    definition: TypeScriptMicroDefinition,
    adapter: std::sync::Arc<dyn TypeScriptHostAdapter>,
}

impl TypeScriptMicroImplementation {
    /// Creates a host implementation from metadata and an adapter.
    pub fn new(
        definition: TypeScriptMicroDefinition,
        adapter: std::sync::Arc<dyn TypeScriptHostAdapter>,
    ) -> Self {
        Self { definition, adapter }
    }
}

impl UdfImplementation for TypeScriptMicroImplementation {
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
