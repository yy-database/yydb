//! Normalized UDF errors for registry, invocation, and host adapters.

use std::fmt;

/// Errors produced by the YYDB UDF subsystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UdfError {
    /// No UDF matches the requested identity.
    NotFound,
    /// A registration collides with an existing entry.
    NameConflict,
    /// Invocation arguments do not match the registered signature.
    SignatureMismatch,
    /// The requested implementation version does not match the registry entry.
    VersionMismatch,
    /// The UDF definition is structurally invalid.
    InvalidDefinition,
    /// A value or type is outside the supported UDF subset.
    UnsupportedType,
    /// The declared effect is not allowed for this registration path.
    UnsupportedEffect,
    /// The host denied a required capability.
    CapabilityDenied,
    /// Catalog requires an implementation that the current host did not register.
    ImplementationUnavailable,
    /// The TypeScript host disconnected or is unavailable.
    HostDisconnected,
    /// The invocation exceeded its time budget.
    Timeout,
    /// The invocation was cancelled.
    Cancelled,
    /// The invocation exceeded a resource budget.
    BudgetExceeded,
    /// The implementation returned an invalid value.
    InvalidReturnValue,
    /// The caller supplied the wrong number of arguments.
    ArityMismatch,
    /// A deterministic UDF observed non-deterministic behavior.
    DeterminismViolation,
    /// The implementation failed during execution.
    ExecutionFailed {
        /// Sanitized message safe for public surfaces.
        message: String,
    },
}

impl fmt::Display for UdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => write!(f, "udf not found"),
            Self::NameConflict => write!(f, "udf name conflict"),
            Self::SignatureMismatch => write!(f, "udf signature mismatch"),
            Self::VersionMismatch => write!(f, "udf version mismatch"),
            Self::InvalidDefinition => write!(f, "invalid udf definition"),
            Self::UnsupportedType => write!(f, "unsupported udf type"),
            Self::UnsupportedEffect => write!(f, "unsupported udf effect"),
            Self::CapabilityDenied => write!(f, "udf capability denied"),
            Self::ImplementationUnavailable => write!(f, "udf implementation unavailable"),
            Self::HostDisconnected => write!(f, "udf host disconnected"),
            Self::Timeout => write!(f, "udf invocation timed out"),
            Self::Cancelled => write!(f, "udf invocation cancelled"),
            Self::BudgetExceeded => write!(f, "udf budget exceeded"),
            Self::InvalidReturnValue => write!(f, "udf returned invalid value"),
            Self::ArityMismatch => write!(f, "udf arity mismatch"),
            Self::DeterminismViolation => write!(f, "udf determinism violation"),
            Self::ExecutionFailed { message } => write!(f, "udf execution failed: {message}"),
        }
    }
}

impl std::error::Error for UdfError {}

/// Convenience result alias for UDF operations.
pub type Result<T> = std::result::Result<T, UdfError>;
