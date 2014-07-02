use std::fs;
use std::path::{Path, PathBuf};

use crate::journal::{shm_path, wal_path};
use crate::{Error, Result};

use super::Connection;

impl Connection {
    /// Copy a self-contained `.yydb` main file to `dest` after checkpoint.
    ///
    /// WAL mode connections are checkpointed first so the backup needs no `-wal` /
    /// `-shm` sidecars (Living `07` §7).
    pub fn backup_to(&self, dest: impl AsRef<Path>) -> Result<()> {
        self.checkpoint()?;
        let src = self
            .path()
            .ok_or(Error::Unsupported("backup_to requires a file-backed connection"))?;
        let dest = dest.as_ref();
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, dest)?;
        Ok(())
    }

    /// Copy the main file and any present `-wal` / `-shm` sidecars without checkpoint.
    ///
    /// Opening `dest` replays the copied WAL tail against the main image.
    pub fn backup_with_wal_to(&self, dest: impl AsRef<Path>) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let src = self
            .path()
            .ok_or(Error::Unsupported(
                "backup_with_wal_to requires a file-backed connection",
            ))?;
        let dest = dest.as_ref();
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, dest)?;
        copy_sidecar_if_present(&wal_path(src), wal_path(dest))?;
        copy_sidecar_if_present(&shm_path(src), shm_path(dest))?;
        Ok(())
    }
}

fn copy_sidecar_if_present(src: &Path, dest: PathBuf) -> Result<()> {
    if src.exists() {
        fs::copy(src, dest)?;
    }
    Ok(())
}
