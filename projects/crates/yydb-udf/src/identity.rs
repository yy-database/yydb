//! Logical UDF identity separate from host implementation objects.

use crate::capability::{Determinism, Effect, Placement};
use crate::contract::Signature;
use crate::error::{Result, UdfError};

/// Stable logical identity for a catalog or session UDF.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UdfIdentity {
    /// Optional namespace prefix (`math`, `text`, …).
    pub namespace: String,
    /// Function name within the namespace.
    pub name: String,
    /// Monotonic logical version.
    pub version: u32,
}

impl UdfIdentity {
    /// Creates a new identity after validating non-empty fields.
    pub fn new(namespace: impl Into<String>, name: impl Into<String>, version: u32) -> Result<Self> {
        let namespace = namespace.into();
        let name = name.into();
        if name.is_empty() {
            return Err(UdfError::InvalidDefinition);
        }
        if version == 0 {
            return Err(UdfError::VersionMismatch);
        }
        Ok(Self {
            namespace,
            name,
            version,
        })
    }

    /// Returns the bare function name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns `namespace/name@version`.
    pub fn logical_id(&self) -> String {
        if self.namespace.is_empty() {
            format!("{}@{}", self.name, self.version)
        } else {
            format!("{}/{}@{}", self.namespace, self.name, self.version)
        }
    }

    /// Returns the canonical `yydb://` URI form.
    pub fn uri(&self) -> String {
        if self.namespace.is_empty() {
            format!("yydb://{}@{}", self.name, self.version)
        } else {
            format!("yydb://{}/{}@{}", self.namespace, self.name, self.version)
        }
    }
}

/// Fingerprint inputs used for plan cache invalidation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UdfFingerprint {
    /// Logical identity.
    pub identity: UdfIdentity,
    /// Argument and return contract.
    pub signature: Signature,
    /// Declared effect.
    pub effect: Effect,
    /// Determinism contract.
    pub determinism: Determinism,
    /// Placement requirement.
    pub placement: Placement,
    /// Source or implementation fingerprint.
    pub implementation: [u8; 32],
}
