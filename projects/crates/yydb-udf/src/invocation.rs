//! Invocation envelopes and bounded execution context.

use crate::error::{Result, UdfError};
use crate::identity::UdfIdentity;
use crate::value::UdfValue;

/// Scalar or batch invocation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationMode {
    /// One row or one logical call at a time.
    Scalar,
    /// Host receives a bounded batch (Phase 2 for TS).
    Batch,
}

/// Explicit UDF invocation command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdfInvocation {
    /// Resolved logical identity.
    pub identity: UdfIdentity,
    /// Positional arguments in boundary form.
    pub args: Vec<UdfValue>,
    /// Invocation mode requested by the planner or host.
    pub mode: InvocationMode,
}

impl UdfInvocation {
    /// Creates a scalar invocation.
    pub fn scalar(identity: UdfIdentity, args: Vec<UdfValue>) -> Self {
        Self {
            identity,
            args,
            mode: InvocationMode::Scalar,
        }
    }

    /// Verifies that the argument count matches `expected_arity`.
    pub fn ensure_arity(&self, expected_arity: usize) -> Result<()> {
        if self.args.len() != expected_arity {
            return Err(UdfError::ArityMismatch);
        }
        Ok(())
    }
}

/// Bounded resources granted to one UDF call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// Remaining abstract cost units for this invocation.
    pub remaining: u64,
}

impl Budget {
    /// Creates a budget with `remaining` units.
    pub fn new(remaining: u64) -> Self {
        Self { remaining }
    }

    /// Charges `units` from the budget.
    pub fn charge(&mut self, units: u64) -> Result<()> {
        if units > self.remaining {
            return Err(UdfError::BudgetExceeded);
        }
        self.remaining -= units;
        Ok(())
    }
}

/// Capability object passed into UDF implementations.
#[derive(Debug, Clone)]
pub struct UdfContext {
    /// Whether the caller requested cancellation.
    pub cancelled: bool,
    /// Remaining invocation budget.
    pub budget: Budget,
}

impl UdfContext {
    /// Creates a fresh invocation context.
    pub fn new(budget: Budget) -> Self {
        Self {
            cancelled: false,
            budget,
        }
    }

    /// Returns an error when cancellation was requested.
    pub fn check_cancelled(&self) -> Result<()> {
        if self.cancelled {
            return Err(UdfError::Cancelled);
        }
        Ok(())
    }

    /// Charges abstract cost units from the invocation budget.
    pub fn charge(&mut self, units: u64) -> Result<()> {
        self.budget.charge(units)
    }
}
