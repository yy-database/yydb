use crate::lease;
use crate::Batch;
use crate::{CommitSequence, LeaseExpectation, LeaseToken, ReleaseOutcome, Result};

use super::Connection;

impl Connection {
    /// Claim a lease on `key` for `worker_id`.
    pub fn claim_lease(
        &self,
        key: &str,
        worker_id: &str,
        lease_duration: std::time::Duration,
        expectation: LeaseExpectation,
    ) -> Result<LeaseToken> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut state = self.read_state()?;
        let token = lease::claim_lease(
            &mut state.records,
            key,
            worker_id,
            lease_duration.as_millis() as u64,
            expectation,
        )?;
        self.write_state(&state)?;
        Ok(token)
    }

    /// Extend the lease deadline for an active token.
    pub fn renew_lease(&self, token: &LeaseToken, new_lease_until: u64) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut state = self.read_state()?;
        lease::renew_lease(&mut state.records, token, new_lease_until)?;
        self.write_state(&state)
    }

    /// Release a lease with a store-visible outcome.
    pub fn release_lease(&self, token: &LeaseToken, outcome: ReleaseOutcome) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut state = self.read_state()?;
        lease::release_lease(&mut state.records, token, outcome)?;
        self.write_state(&state)
    }

    /// Apply `batch` only when `token` matches the active claim.
    pub fn commit_with_lease(&self, token: &LeaseToken, batch: Batch) -> Result<CommitSequence> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut state = self.read_state()?;
        lease::verify_token(&state.records, token)?;
        for (key, value) in batch.changes {
            match value {
                Some(bytes) => {
                    state.records.insert(key, bytes);
                }
                None => {
                    state.records.remove(&key);
                }
            }
        }
        let sequence = lease::bump_commit_sequence(&mut state.records);
        lease::release_lease(&mut state.records, token, ReleaseOutcome::Requeue)?;
        self.write_state(&state)?;
        Ok(sequence)
    }

    /// Read the active lease deadline for `key`, if any.
    pub fn lease_until_millis(&self, key: &str) -> Result<Option<u64>> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        lease::lease_until(&self.read_state()?.records, key)
    }
}
