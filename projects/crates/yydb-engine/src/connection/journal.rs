use crate::journal::JournalMode;
use crate::storage::doctor;
use crate::Result;

use super::{Backend, Connection};

impl Connection {
    /// Current journal mode (`delete` or `wal`). In-memory is always `delete`.
    pub fn journal_mode(&self) -> JournalMode {
        match self.backend.as_ref() {
            Backend::File { database, .. } => database
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .journal_mode(),
            Backend::Memory { .. } => JournalMode::Delete,
        }
    }

    /// Switch journal mode. Enabling WAL creates sidecars; disabling WAL
    /// checkpoints then removes `{db}-wal` / `{db}-shm`.
    pub fn set_journal_mode(&self, mode: JournalMode) -> Result<()> {
        match self.backend.as_ref() {
            Backend::Memory { .. } => Ok(()),
            Backend::File { database, .. } => database
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_journal_mode(mode),
        }
    }

    /// Fold WAL frames into the main file and truncate `-wal` / reset `-shm`.
    ///
    /// No-op when not in WAL mode or when there is nothing to fold.
    pub fn checkpoint(&self) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        match self.backend.as_ref() {
            Backend::Memory { .. } => Ok(()),
            Backend::File { database, .. } => {
                if self.journal_mode() != JournalMode::Wal {
                    return Ok(());
                }
                let mut database = database
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let mut state =
                    crate::storage::snapshot::state_from_snapshot(&database.snapshot()?)?;
                doctor::record_checkpoint(&mut state.records);
                let next = crate::storage::snapshot::snapshot_from_state(&state)?;
                database.commit(&next, &[])?;
                database.checkpoint()
            }
        }
    }

    /// Number of committed frames recorded in `-shm` (0 if absent).
    pub fn wal_frame_count(&self) -> Result<u32> {
        match self.backend.as_ref() {
            Backend::File { database, .. } => database
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .wal_frame_count(),
            Backend::Memory { .. } => Ok(0),
        }
    }

    pub(crate) fn wal_bytes(&self) -> u64 {
        match self.backend.as_ref() {
            Backend::File { database, .. } => database
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .wal_bytes(),
            Backend::Memory { .. } => 0,
        }
    }
}
