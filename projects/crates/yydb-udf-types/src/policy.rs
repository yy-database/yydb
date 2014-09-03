//! Effect, capability, and placement contracts for UDF registration.

/// Declared side-effect class for a UDF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Effect {
    /// Pure scalar transformation.
    Pure,
    /// Reads local database state.
    ReadLocal,
    /// Reads files through an explicit host capability.
    ReadFile,
    /// Reads process environment.
    ReadEnvironment,
    /// Performs network I/O.
    Network,
    /// Mutates local host state outside the database.
    WriteLocal,
    /// Arbitrary external side effects.
    ExternalSideEffect,
}

/// Capability flags granted to a UDF invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CapabilitySet {
    /// May read files.
    pub file_read: bool,
    /// May write files.
    pub file_write: bool,
    /// May use the network.
    pub network: bool,
    /// May read environment variables.
    pub environment: bool,
    /// May read the clock.
    pub clock: bool,
    /// May use randomness.
    pub random: bool,
}

/// Whether evaluation is safe to repeat with identical inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Determinism {
    /// Repeated evaluation must yield the same result.
    Deterministic,
    /// Evaluation may vary between calls.
    NonDeterministic,
}

/// Where the UDF body is allowed to execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Placement {
    /// Lowered program or native code in the embedded database process.
    Embedded,
    /// Host callback outside the core execution loop.
    Host,
}

/// Registration-time execution policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UdfPolicy {
    /// Declared effect.
    pub effect: Effect,
    /// Granted capabilities.
    pub capabilities: CapabilitySet,
    /// Determinism contract.
    pub determinism: Determinism,
    /// Whether failed pure calls may be retried automatically.
    pub retryable: bool,
    /// Whether identical invocations may be cached.
    pub cacheable: bool,
}

impl UdfPolicy {
    /// Returns a pure embedded policy suitable for Phase 1 scalar UDFs.
    pub fn pure_embedded() -> Self {
        Self {
            effect: Effect::Pure,
            capabilities: CapabilitySet::default(),
            determinism: Determinism::Deterministic,
            retryable: true,
            cacheable: true,
        }
    }

    /// Returns a pure host policy for session host micros.
    pub fn pure_host() -> Self {
        Self {
            effect: Effect::Pure,
            capabilities: CapabilitySet::default(),
            determinism: Determinism::Deterministic,
            retryable: false,
            cacheable: false,
        }
    }

    /// Returns whether row-at-a-time execution is allowed by default.
    pub fn allows_scalar_row_execution(&self) -> bool {
        matches!(
            self.effect,
            Effect::Pure | Effect::ReadLocal | Effect::ReadFile
        )
    }
}
