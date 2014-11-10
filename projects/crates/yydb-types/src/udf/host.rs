//! Transportable host micro metadata shared by protocol and runtime.

use super::{
    Result, UdfError,
    identity::UdfIdentity,
    policy::{Placement, UdfPolicy},
    signature::{ImplementationKind, Signature, UdfDefinition, UdfType},
};

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
    #[allow(clippy::too_many_arguments)]
    pub fn from_scalar(
        name: &str,
        version: u32,
        args: Vec<UdfType>,
        returns: UdfType,
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
            handle: HostFunctionHandle { host_id, function_id, version: handle_version },
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
