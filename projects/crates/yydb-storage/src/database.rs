//! File-backed database lifecycle: open, snapshot commit, WAL coordination.

use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

use yydb_formats::{
    BlobManifestDelta, FilePager, MemoryPager, PAGE_MAGIC, WAL_MAGIC, main_bytes_from_memory_pager,
    memory_pager_from_main_bytes, parse_wal,
};
use yydb_types::{Error, Result};

use crate::{
    journal::{JournalMode, wal_path},
    lock::WriterLock,
    snapshot::{Snapshot, read_snapshot, write_snapshot},
};

/// An open, exclusively locked file-backed database image.
pub struct Database {
    path: PathBuf,
    _writer_lock: WriterLock,
    pager: FilePager,
    journal_mode: JournalMode,
}

impl Database {
    /// Open (or create) a database file and hold the cross-process writer lock.
    ///
    /// A leftover `YYWL` v0 sidecar is folded into the main file when the caller
    /// asked for `delete` journal mode.
    pub fn open(path: impl AsRef<Path>, journal_mode: JournalMode) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if path.exists() && !file_is_ydpg(&path)? {
            return Err(Error::Corrupt("main file is not YDPG format v0"));
        }
        let writer_lock = WriterLock::acquire(&path)?;
        let enable_wal = journal_mode == JournalMode::Wal;
        let pager = if path.exists() && wal_path(&path).exists() {
            let mut pager = FilePager::open(&path, true)?;
            if !enable_wal {
                pager.checkpoint()?;
                pager.disable_wal()?;
            }
            pager
        }
        else {
            FilePager::open(&path, enable_wal)?
        };
        Ok(Self { path, _writer_lock: writer_lock, pager, journal_mode })
    }

    /// Filesystem path of the main database image.
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    /// Current journal mode.
    pub fn journal_mode(&self) -> JournalMode {
        self.journal_mode
    }

    /// Switch journal mode, creating or checkpointing sidecars as needed.
    pub fn set_journal_mode(&mut self, mode: JournalMode) -> Result<()> {
        if self.journal_mode == mode {
            return Ok(());
        }
        match mode {
            JournalMode::Wal => self.pager.enable_wal()?,
            JournalMode::Delete => self.pager.disable_wal()?,
        }
        self.journal_mode = mode;
        Ok(())
    }

    /// Load the current durable snapshot.
    pub fn snapshot(&mut self) -> Result<Snapshot> {
        read_snapshot(&mut self.pager)
    }

    /// Commit a snapshot, publishing `blob_refs` into the WAL transaction when set.
    pub fn commit(&mut self, snapshot: &Snapshot, blob_refs: &[BlobManifestDelta]) -> Result<()> {
        self.pager.mutate_with_publish_blob_refs(|inner| write_snapshot(inner, snapshot), blob_refs)
    }

    /// Fold WAL frames into the main file.
    pub fn checkpoint(&mut self) -> Result<()> {
        self.pager.checkpoint()
    }

    /// Committed frame count recorded in the `-wal` sidecar.
    pub fn wal_frame_count(&self) -> Result<u32> {
        let wal = wal_path(&self.path);
        if !wal.exists() {
            return Ok(0);
        }
        let bytes = fs::read(&wal)?;
        if bytes.get(0..5) != Some(WAL_MAGIC.as_slice()) {
            return Err(Error::Corrupt("wal magic mismatch"));
        }
        Ok(parse_wal(&bytes)?.frames.len() as u32)
    }

    /// Uncheckpointed WAL bytes on disk (0 when the sidecar is absent).
    pub fn wal_bytes(&self) -> u64 {
        fs::metadata(wal_path(&self.path)).map(|meta| meta.len()).unwrap_or(0)
    }
}

/// True when `path` exists and begins with `YDPG` page magic.
pub fn file_is_ydpg(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let mut file = File::open(path)?;
    let mut magic = [0_u8; 5];
    let read = file.read(&mut magic)?;
    Ok(read == PAGE_MAGIC.len() && magic == *PAGE_MAGIC)
}

/// Render a snapshot as a checkpointed in-memory `YDPG` main-file image.
pub fn snapshot_to_main_bytes(snapshot: &Snapshot, database_id: [u8; 16], page_size: u8) -> Result<Vec<u8>> {
    let mut pager = MemoryPager::new_empty(database_id, page_size);
    write_snapshot(&mut pager, snapshot)?;
    main_bytes_from_memory_pager(&pager)
}

/// Read a snapshot from an in-memory `YDPG` main-file image.
pub fn snapshot_from_main_bytes(bytes: &[u8]) -> Result<Snapshot> {
    let mut pager = memory_pager_from_main_bytes(bytes)?;
    read_snapshot(&mut pager)
}
