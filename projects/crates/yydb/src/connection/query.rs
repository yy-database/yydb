use crate::query;
use crate::udf::ScalarUdf;
use crate::{Error, Result};

use super::Connection;

impl Connection {
    /// Execute a Phase 1 VOS read pipeline (for example `User.filter(x => x.active).collect()`).
    pub fn query(&self, source: &str) -> Result<Vec<query::QueryRow>> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut state = self.active_state_unlocked()?;
        if let Some(catalog) = state.catalog.as_ref() {
            if let Some(rows) = query::try_insert_returning(source, catalog, &mut state.records)? {
                self.replace_active_state_unlocked(state)?;
                return Ok(rows);
            }
        }
        query::execute(source, state.catalog.as_ref(), &state.records)
    }

    /// Execute unit-valued VOS write programs (for example `User { … }.insert()`).
    pub fn execute(&self, source: &str) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        self.mutate_active_state_unlocked(|state| {
            let catalog = state.catalog.as_ref().ok_or_else(|| Error::Schema {
                message: "call ensure_schema before execute".into(),
            })?;
            let schema_document = state.schema.as_ref().map(|schema| schema.document.as_str());
            query::execute_write(source, catalog, schema_document, &mut state.records)
        })
    }

    /// Begin a data transaction on this connection.
    ///
    /// While open, `query` / `execute` / `upsert_row` read and write against an
    /// in-memory snapshot. [`Self::commit`] persists the snapshot, and
    /// [`Self::rollback`] discards it.
    pub fn begin(&self) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut slot = self
            .txn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.is_some() {
            return Err(Error::Schema {
                message: "transaction already open".into(),
            });
        }
        *slot = Some(self.read_state()?);
        Ok(())
    }

    /// Commit the open data transaction.
    pub fn commit(&self) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut slot = self
            .txn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(state) = slot.take() else {
            return Err(Error::Schema {
                message: "no open transaction".into(),
            });
        };
        drop(slot);
        self.write_state(&state)
    }

    /// Roll back the open data transaction.
    pub fn rollback(&self) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut slot = self
            .txn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.is_none() {
            return Err(Error::Schema {
                message: "no open transaction".into(),
            });
        }
        *slot = None;
        Ok(())
    }

    /// Whether a data transaction is open on this connection.
    pub fn in_transaction(&self) -> bool {
        self.txn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }

    /// Upsert one logical table row used by the Phase 1 query executor.
    pub fn upsert_row(
        &self,
        table: impl AsRef<str>,
        pk: impl AsRef<str>,
        row: query::QueryRow,
    ) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        self.mutate_active_state_unlocked(|state| {
            query::upsert_row(&mut state.records, table.as_ref(), pk.as_ref(), &row)
        })
    }
}
