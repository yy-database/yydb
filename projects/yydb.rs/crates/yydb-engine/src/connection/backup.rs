use std::path::Path;

use yydb_storage::files::{copy_main, copy_sidecars, copy_tree};

use crate::{
    Error, Result,
    objects::{ObjectStore, references as refs},
};

use super::{Connection, is_yydx_main_path};

impl Connection {
    /// Copy a self-contained `.yydb` main file to `dest` after checkpoint.
    ///
    /// WAL mode connections are checkpointed first so the backup needs no `-wal` /
    /// `-shm` sidecars (Living `07` §7).
    pub fn backup_to(&self, dest: impl AsRef<Path>) -> Result<()> {
        self.checkpoint()?;
        let src = self.path().ok_or(Error::Unsupported("backup_to requires a file-backed connection"))?;
        copy_main(src, dest.as_ref())
    }

    /// Copy the main file and any present `-wal` / `-shm` sidecars without checkpoint.
    ///
    /// Opening `dest` replays the copied WAL tail against the main image.
    pub fn backup_with_wal_to(&self, dest: impl AsRef<Path>) -> Result<()> {
        let _guard = self.operation_lock.lock().unwrap_or_else(|p| p.into_inner());
        let src = self.path().ok_or(Error::Unsupported("backup_with_wal_to requires a file-backed connection"))?;
        let dest = dest.as_ref();
        copy_main(src, dest)?;
        copy_sidecars(src, dest)
    }

    /// Copy a checkpointed `.yydx` main file and sibling `<stem>-objects/` tree to `dest`.
    ///
    /// `dest` must use the `.yydx` suffix. Sidecars are folded via checkpoint so the backup
    /// is self-contained (Living `07` §7–§8).
    pub fn backup_yydx_to(&self, dest: impl AsRef<Path>) -> Result<()> {
        let src = self.path().ok_or(Error::Unsupported("backup_yydx_to requires a file-backed connection"))?;
        if !is_yydx_main_path(src) {
            return Err(Error::Unsupported("backup_yydx_to requires a .yydx main file path"));
        }
        let dest = dest.as_ref();
        if !is_yydx_main_path(dest) {
            return Err(Error::Unsupported("backup_yydx_to destination must use the .yydx suffix"));
        }
        self.checkpoint()?;
        copy_main(src, dest)?;
        copy_tree(&ObjectStore::yydx_objects_root(src), &ObjectStore::yydx_objects_root(dest))
    }

    /// Open a restored `.yydx` backup and verify every committed `file` value.
    ///
    /// Restore is only complete once each distinct file descriptor has been read
    /// back in full with its exact digest (Living `03` §3.3). A missing object
    /// or a hash mismatch fails the call closed instead of handing back a
    /// connection that would surface the gap as corrupt reads later.
    pub fn restore_yydx(backup: impl AsRef<Path>) -> Result<Self> {
        let connection = Self::open_yydx(backup)?;
        connection.verify_file_digests()?;
        Ok(connection)
    }

    /// Read every committed `file` value in full and verify its digest.
    ///
    /// Returns the number of distinct file values checked. The SHA-256 digest
    /// covers the complete octets and the manifest is content-addressed, so a
    /// reordered, truncated, or swapped chunk set fails here instead of reading
    /// back as plausible-looking bytes.
    pub fn verify_file_digests(&self) -> Result<usize> {
        let _guard = self.operation_lock.lock().unwrap_or_else(|p| p.into_inner());
        let state = self.read_state()?;
        let descriptors = refs::file_descriptors(&state.records, "")?;
        for descriptor in &descriptors {
            self.objects.read_file(descriptor)?;
        }
        Ok(descriptors.len())
    }
}
