use std::path::Path;

use yydb_storage::files::{copy_main, copy_sidecars, copy_tree};

use crate::objects::ObjectStore;
use crate::{Error, Result};

use super::is_yydx_main_path;
use super::Connection;

impl Connection {
    /// Copy a self-contained `.yydb` main file to `dest` after checkpoint.
    ///
    /// WAL mode connections are checkpointed first so the backup needs no `-wal` /
    /// `-shm` sidecars (Living `07` §7).
    pub fn backup_to(&self, dest: impl AsRef<Path>) -> Result<()> {
        self.checkpoint()?;
        let src = self.path().ok_or(Error::Unsupported(
            "backup_to requires a file-backed connection",
        ))?;
        copy_main(src, dest.as_ref())
    }

    /// Copy the main file and any present `-wal` / `-shm` sidecars without checkpoint.
    ///
    /// Opening `dest` replays the copied WAL tail against the main image.
    pub fn backup_with_wal_to(&self, dest: impl AsRef<Path>) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let src = self.path().ok_or(Error::Unsupported(
            "backup_with_wal_to requires a file-backed connection",
        ))?;
        let dest = dest.as_ref();
        copy_main(src, dest)?;
        copy_sidecars(src, dest)
    }

    /// Copy a checkpointed `.yydx` main file and sibling `<stem>-objects/` tree to `dest`.
    ///
    /// `dest` must use the `.yydx` suffix. Sidecars are folded via checkpoint so the backup
    /// is self-contained (Living `07` §7–§8).
    pub fn backup_yydx_to(&self, dest: impl AsRef<Path>) -> Result<()> {
        let src = self.path().ok_or(Error::Unsupported(
            "backup_yydx_to requires a file-backed connection",
        ))?;
        if !is_yydx_main_path(src) {
            return Err(Error::Unsupported(
                "backup_yydx_to requires a .yydx main file path",
            ));
        }
        let dest = dest.as_ref();
        if !is_yydx_main_path(dest) {
            return Err(Error::Unsupported(
                "backup_yydx_to destination must use the .yydx suffix",
            ));
        }
        self.checkpoint()?;
        copy_main(src, dest)?;
        copy_tree(
            &ObjectStore::yydx_objects_root(src),
            &ObjectStore::yydx_objects_root(dest),
        )
    }
}
