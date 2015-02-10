use crate::{Result, journal::JournalMode, kv::lease, storage::snapshot};

use super::{Backend, Connection, State, is_yydx_main_path};

impl Connection {
    pub(crate) fn reconcile_leases_on_open(&self) -> Result<()> {
        let _guard = self.operation_lock.lock().unwrap_or_else(|p| p.into_inner());
        self.mutate_active_state_unlocked(|state| {
            lease::reconcile_expired_leases(&mut state.records)?;
            Ok(())
        })
    }

    pub(crate) fn active_state_unlocked(&self) -> Result<State> {
        if let Some(state) = self.txn.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref() {
            Ok(state.clone())
        }
        else {
            self.read_state()
        }
    }

    pub(crate) fn replace_active_state_unlocked(&self, state: State) -> Result<()> {
        let mut slot = self.txn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.is_some() {
            *slot = Some(state);
            Ok(())
        }
        else {
            drop(slot);
            self.write_state(&state)
        }
    }

    pub(crate) fn mutate_active_state_unlocked<F>(&self, f: F) -> Result<()>
    where
        F: FnOnce(&mut State) -> Result<()>,
    {
        let mut state = self.active_state_unlocked()?;
        f(&mut state)?;
        self.replace_active_state_unlocked(state)
    }

    pub(crate) fn read_state(&self) -> Result<State> {
        match self.backend.as_ref() {
            Backend::File { database, .. } => {
                let mut database = database.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                snapshot::state_from_snapshot(&database.snapshot()?)
            }
            Backend::Memory { state } => Ok(state.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()),
        }
    }

    pub(crate) fn write_state(&self, state: &State) -> Result<()> {
        match self.backend.as_ref() {
            Backend::File { database, .. } => {
                let blob_objects = if self.path().is_some_and(is_yydx_main_path) && self.journal_mode() == JournalMode::Wal {
                    Some(self.objects())
                }
                else {
                    None
                };
                let mut database = database.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                let before = database.snapshot()?.records;
                let next = snapshot::snapshot_from_state(state)?;
                let blob_refs = match blob_objects {
                    Some(objects) => snapshot::blob_deltas(&before, &next.records, objects)?,
                    None => Vec::new(),
                };
                database.commit(&next, &blob_refs)
            }
            Backend::Memory { state: slot } => {
                *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = state.clone();
                Ok(())
            }
        }
    }
}
