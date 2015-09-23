use crate::{Result, journal::JournalMode, kv::lease, objects::EMBEDDED_OBJECT_PREFIX, storage::snapshot};
use yydb_storage::snapshot::RecordWindow;

use super::{Backend, Connection, State, is_yydx_main_path};

impl Connection {
    pub(crate) fn reconcile_leases_on_open(&self) -> Result<()> {
        let _guard = self.operation_lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut state = self.active_state_unlocked()?;
        if lease::reconcile_expired_leases(&mut state.records)? {
            self.replace_active_state_unlocked(state)?;
        }
        Ok(())
    }

    pub(crate) fn active_state_unlocked(&self) -> Result<State> {
        if let Some(state) = self.txn.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref() {
            Ok(state.clone())
        }
        else {
            self.read_state()
        }
    }

    /// Whether a transaction snapshot is currently open.
    pub(crate) fn has_open_transaction(&self) -> bool {
        self.txn.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).is_some()
    }

    pub(crate) fn replace_active_state_unlocked(&self, state: State) -> Result<()> {
        let mut slot = self.txn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.is_some() {
            *slot = Some(state);
            Ok(())
        }
        else {
            drop(slot);
            self.write_state(&state)?;
            Ok(())
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

    /// Read reserved metadata only and decode the schema and contract without
    /// materializing a single table row.
    ///
    /// Reads use this as the first phase: the decoded contract tells the query
    /// layer which record-key window one statement can actually reach.
    pub(crate) fn read_state_metadata(&self) -> Result<State> {
        match self.backend.as_ref() {
            Backend::File { database, .. } => {
                let mut database = database.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                snapshot::state_from_snapshot(&database.snapshot_windows(&[])?)
            }
            Backend::Memory { state } => Ok(state.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()),
        }
    }

    /// Read metadata plus the rows inside `window` in one pager transaction.
    ///
    /// The window is a **superset** of what the statement may read: execution
    /// still applies every filter, so an over-wide window can only cost extra
    /// reads and never change the result. Single-file `.yydb` images also pull
    /// the embedded object prefix, because
    /// [`Self::read_state`] would otherwise hand the resolver an incomplete
    /// object catalog.
    pub(crate) fn read_state_window(&self, window: &yydb_query::ReadWindow) -> Result<State> {
        match self.backend.as_ref() {
            Backend::File { database, .. } => {
                let mut windows = vec![RecordWindow { start: window.start.clone(), end: window.end.clone() }];
                if self.is_single_file_yydb() {
                    windows.push(RecordWindow {
                        start: EMBEDDED_OBJECT_PREFIX.to_owned(),
                        end: Some(format!("{EMBEDDED_OBJECT_PREFIX}g")),
                    });
                }
                let snapshot = {
                    let mut database = database.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    database.snapshot_windows(&windows)?
                };
                let mut state = snapshot::state_from_snapshot(&snapshot)?;
                if self.is_single_file_yydb() {
                    self.objects.take_embedded_records(&mut state.records)?;
                }
                Ok(state)
            }
            Backend::Memory { state } => Ok(state.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()),
        }
    }

    pub(crate) fn read_state(&self) -> Result<State> {
        let mut state = match self.backend.as_ref() {
            Backend::File { database, .. } => {
                let mut database = database.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                snapshot::state_from_snapshot(&database.snapshot()?)?
            }
            Backend::Memory { state } => state.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone(),
        };
        if self.is_single_file_yydb() {
            self.objects.take_embedded_records(&mut state.records)?;
        }
        Ok(state)
    }

    /// Publish a state snapshot, advancing the durable commit sequence by one.
    ///
    /// This is the single publish point for every durable state change, so the
    /// `__yydb/meta/commit_sequence` record doubles as the transaction fencing
    /// base: `begin` captures it and `commit` refuses to publish once it moved.
    pub(crate) fn write_state(&self, state: &State) -> Result<yydb_types::CommitSequence> {
        let mut state = state.clone();
        if self.is_single_file_yydb() {
            self.objects.embed_into(&mut state.records)?;
        }
        let sequence = lease::bump_commit_sequence(&mut state.records);
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
                let next = snapshot::snapshot_from_state(&state)?;
                let blob_refs = match blob_objects {
                    Some(objects) => snapshot::blob_deltas(&before, &next.records, objects)?,
                    None => Vec::new(),
                };
                database.commit(&next, &blob_refs)?;
            }
            Backend::Memory { state: slot } => {
                *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = state;
            }
        }
        Ok(sequence)
    }
}
