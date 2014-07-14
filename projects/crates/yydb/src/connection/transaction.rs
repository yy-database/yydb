use crate::{Error, Result};

use super::Connection;

impl Connection {
    /// Begin a KV-state transaction on this connection.
    ///
    /// While open, state reads and writes go through an in-memory snapshot.
    /// [`Self::commit`] persists the snapshot, and [`Self::rollback`] discards it.
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

    /// Commit the open transaction.
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

    /// Roll back the open transaction.
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

    /// Whether a transaction is open on this connection.
    pub fn in_transaction(&self) -> bool {
        self.txn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }
}
