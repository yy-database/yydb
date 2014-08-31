use std::fs;

use crate::doctor;
use crate::format_v0;
use crate::journal::{wal_path, JournalMode};
use crate::{Error, Result};
use yydb_format::{parse_wal, WAL_MAGIC};

use super::{Backend, Connection};

impl Connection {
    /// Current journal mode (`delete` or `wal`). In-memory is always `delete`.
    pub fn journal_mode(&self) -> JournalMode {
        match &self.backend {
            Backend::File { journal_mode, .. } => *journal_mode
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            Backend::Memory { .. } => JournalMode::Delete,
        }
    }

    /// Switch journal mode. Enabling WAL creates sidecars; disabling WAL
    /// checkpoints then removes `{db}-wal` / `{db}-shm`.
    pub fn set_journal_mode(&self, mode: JournalMode) -> Result<()> {
        match &self.backend {
            Backend::Memory { .. } => return Ok(()),
            Backend::File {
                pager,
                journal_mode,
                ..
            } => {
                let mut slot = journal_mode
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if *slot == mode {
                    return Ok(());
                }
                let mut pager = pager
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                match mode {
                    JournalMode::Wal => pager.enable_wal()?,
                    JournalMode::Delete => pager.disable_wal()?,
                }
                *slot = mode;
            }
        }
        Ok(())
    }

    /// Fold WAL frames into the main file and truncate `-wal` / reset `-shm`.
    ///
    /// No-op when not in WAL mode or when there is nothing to fold.
    pub fn checkpoint(&self) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        match &self.backend {
            Backend::Memory { .. } => Ok(()),
            Backend::File {
                pager,
                journal_mode,
                ..
            } => {
                let mode = *journal_mode
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if mode != JournalMode::Wal {
                    return Ok(());
                }
                let mut pager = pager
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let mut state = format_v0::read_state(&mut pager)?;
                doctor::record_checkpoint(&mut state.records);
                format_v0::write_state(&mut pager, &state)?;
                pager.checkpoint()?;
                Ok(())
            }
        }
    }

    /// Number of committed frames recorded in `-shm` (0 if absent).
    pub fn wal_frame_count(&self) -> Result<u32> {
        match &self.backend {
            Backend::File { path, .. } => {
                let wal = wal_path(path);
                if !wal.exists() {
                    return Ok(0);
                }
                let bytes = fs::read(wal)?;
                if bytes.get(0..5) != Some(WAL_MAGIC.as_slice()) {
                    return Err(Error::Corrupt("wal magic mismatch"));
                }
                Ok(parse_wal(&bytes)?.frames.len() as u32)
            }
            Backend::Memory { .. } => Ok(0),
        }
    }
}
