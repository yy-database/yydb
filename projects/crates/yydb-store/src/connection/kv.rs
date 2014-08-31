use crate::lease;
use crate::ttl;
use crate::{EvictBudget, EvictReport, NamespaceQuota, NamespaceStats, RecordVersion, Result};

use super::Connection;

impl Connection {
    /// Insert or replace a raw byte record keyed by `key`.
    ///
    /// Inline row storage is **allowed** but **not recommended** for large
    /// payloads — prefer [`Self::put_chunk`] / [`Self::put_file_chunked`] into
    /// the unified `objects/` CAS (see `INLINE_BYTES_MAX`).
    pub fn put(&self, key: impl Into<String>, value: impl AsRef<[u8]>) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        self.mutate_active_state_unlocked(|state| {
            ttl::put_record(&mut state.records, &key.into(), value.as_ref())
        })
    }

    /// Store a record that expires after `ttl`.
    pub fn put_with_ttl(
        &self,
        key: impl Into<String>,
        value: impl AsRef<[u8]>,
        ttl: std::time::Duration,
    ) -> Result<RecordVersion> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut state = self.active_state_unlocked()?;
        let version = ttl::put_with_ttl(
            &mut state.records,
            &key.into(),
            value.as_ref(),
            ttl.as_millis() as u64,
        )?;
        self.replace_active_state_unlocked(state)?;
        Ok(version)
    }

    /// Configure quota limits for a namespace prefix such as `cache/response/`.
    pub fn set_namespace_quota(&self, namespace: &str, quota: NamespaceQuota) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        self.mutate_active_state_unlocked(|state| {
            ttl::set_namespace_quota(&mut state.records, namespace, quota);
            Ok(())
        })
    }

    /// Return byte and record usage for a namespace prefix.
    pub fn namespace_stats(&self, namespace: &str) -> Result<NamespaceStats> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        Ok(ttl::namespace_stats(
            &self.active_state_unlocked()?.records,
            namespace,
        ))
    }

    /// Remove expired TTL records under `namespace_prefix` up to `budget`.
    pub fn evict_expired(
        &self,
        namespace_prefix: &str,
        budget: EvictBudget,
    ) -> Result<EvictReport> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut state = self.active_state_unlocked()?;
        let report = ttl::evict_expired(&mut state.records, namespace_prefix, budget)?;
        self.replace_active_state_unlocked(state)?;
        Ok(report)
    }

    /// Fetch a raw byte record by key.
    pub fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let state = self.active_state_unlocked()?;
        if ttl::is_expired(&state.records, key) {
            return Ok(None);
        }
        Ok(state.records.get(key).cloned())
    }

    /// Commit a batch in one journal snapshot. `None` deletes the key.
    /// Operations are serialized on this connection, not across separate handles or processes.
    pub fn write_batch(&self, changes: &[(String, Option<Vec<u8>>)]) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        self.mutate_active_state_unlocked(|state| {
            for (key, value) in changes {
                match value {
                    Some(bytes) => {
                        state.records.insert(key.clone(), bytes.clone());
                    }
                    None => {
                        state.records.remove(key);
                    }
                }
            }
            lease::bump_commit_sequence(&mut state.records);
            Ok(())
        })
    }

    /// Replace or delete a key only if its current bytes match `expected`.
    /// `None` as expected requires an absent key. Atomic on a shared connection.
    pub fn compare_exchange(
        &self,
        key: &str,
        expected: Option<&[u8]>,
        replacement: Option<&[u8]>,
    ) -> Result<bool> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut state = self.active_state_unlocked()?;
        if state.records.get(key).map(Vec::as_slice) != expected {
            return Ok(false);
        }
        match replacement {
            Some(bytes) => {
                state.records.insert(key.to_owned(), bytes.to_vec());
            }
            None => {
                state.records.remove(key);
            }
        }
        self.replace_active_state_unlocked(state)?;
        Ok(true)
    }

    /// Scan keys in lexical order with an exclusive continuation key and a result limit.
    pub fn scan_prefix(
        &self,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(String, Vec<u8>)>> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let state = self.active_state_unlocked()?;
        Ok(state
            .records
            .range(prefix.to_owned()..)
            .take_while(|(key, _)| key.starts_with(prefix))
            .filter(|(key, _)| after.is_none_or(|cursor| key.as_str() > cursor))
            .take(limit)
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect())
    }
}
