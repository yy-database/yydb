use crate::{Error, Result};

use super::Connection;

impl Connection {
    /// Begin a KV-state transaction on this connection.
    ///
    /// While open, KV reads and writes (`get`, `put`, `write_batch`,
    /// `compare_exchange`, `scan_prefix`, …) and VOS DML
    /// ([`Self::execute_vos_write`]) operate on an in-memory snapshot.
    /// [`Self::commit`] persists the snapshot, and [`Self::rollback`] discards
    /// it. Schema and DDL calls stay outside the transaction.
    pub fn begin(&self) -> Result<()> {
        let _guard = self.operation_lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut slot = self.txn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.is_some() {
            return Err(Error::Schema { message: "transaction already open".into() });
        }
        *slot = Some(self.read_state()?);
        Ok(())
    }

    /// Commit the open transaction.
    pub fn commit(&self) -> Result<()> {
        let _guard = self.operation_lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut slot = self.txn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(state) = slot.take()
        else {
            return Err(Error::Schema { message: "no open transaction".into() });
        };
        // Transaction ends — any live savepoints are discarded with it.
        let mut savepoints = self.savepoints.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        savepoints.clear();
        drop(savepoints);
        drop(slot);
        self.write_state(&state)
    }

    /// Roll back the open transaction.
    pub fn rollback(&self) -> Result<()> {
        let _guard = self.operation_lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut slot = self.txn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.is_none() {
            return Err(Error::Schema { message: "no open transaction".into() });
        }
        *slot = None;
        // Transaction ends — any live savepoints are discarded with it.
        let mut savepoints = self.savepoints.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        savepoints.clear();
        Ok(())
    }

    /// Whether a transaction is open on this connection.
    pub fn in_transaction(&self) -> bool {
        self.txn.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).is_some()
    }

    /// Create a named savepoint inside the open transaction.
    ///
    /// Rejects when no transaction is open or when a savepoint with the same
    /// name already exists on this connection.
    pub fn savepoint(&self, name: &str) -> Result<()> {
        let _guard = self.operation_lock.lock().unwrap_or_else(|p| p.into_inner());
        let txn_slot = self.txn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(_) = txn_slot.as_ref()
        else {
            return Err(Error::Schema { message: "no open transaction".into() });
        };
        let mut savepoints = self.savepoints.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if savepoints.iter().any(|(existing, _)| existing == name) {
            return Err(Error::Schema {
                message: format!("savepoint `{name}` already exists"),
            });
        }
        let state = txn_slot.as_ref().expect("txn slot just proved Some").clone();
        savepoints.push((name.to_owned(), state));
        Ok(())
    }

    /// Roll back the transaction to a previously created savepoint.
    ///
    /// Restores the transaction buffer to the snapshot captured at savepoint
    /// creation and removes the savepoint together with every savepoint nested
    /// strictly above it. Fails when no transaction is open or the name is
    /// unknown.
    pub fn rollback_to_savepoint(&self, name: &str) -> Result<()> {
        let _guard = self.operation_lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut slot = self.txn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.is_none() {
            return Err(Error::Schema { message: "no open transaction".into() });
        }
        let mut savepoints = self.savepoints.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let index = savepoints
            .iter()
            .rposition(|(existing, _)| existing == name)
            .ok_or_else(|| Error::Schema {
                message: format!("savepoint `{name}` does not exist"),
            })?;
        let (_, snapshot) = savepoints.remove(index);
        // Drop every savepoint strictly above the named one so later rollback-to
        // cannot jump past its own restore barrier.
        savepoints.truncate(index);
        drop(savepoints);
        *slot = Some(snapshot);
        Ok(())
    }

    /// Release (commit) a named savepoint without rolling anything back.
    ///
    /// Subsequent `ROLLBACK TO SAVEPOINT <name>` will fail. The savepoint is
    /// simply removed from the connection's savepoint stack. Fails when no
    /// transaction is open or the name is unknown.
    pub fn release_savepoint(&self, name: &str) -> Result<()> {
        let _guard = self.operation_lock.lock().unwrap_or_else(|p| p.into_inner());
        let slot = self.txn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.is_none() {
            return Err(Error::Schema { message: "no open transaction".into() });
        }
        let mut savepoints = self.savepoints.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let index = savepoints
            .iter()
            .rposition(|(existing, _)| existing == name)
            .ok_or_else(|| Error::Schema {
                message: format!("savepoint `{name}` does not exist"),
            })?;
        savepoints.remove(index);
        Ok(())
    }
}
