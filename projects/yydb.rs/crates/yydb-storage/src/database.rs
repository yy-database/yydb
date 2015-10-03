//! File-backed database lifecycle: open, snapshot commit, WAL coordination.

use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

use yydb_formats::{
    BlobManifestDelta, FilePager, MemoryPager, PAGE_MAGIC, PublishStats, RecoveryReport, WAL_MAGIC,
    main_bytes_from_memory_pager, memory_pager_from_main_bytes, parse_wal_recover,
};
use yydb_types::{Error, Result};

use crate::{
    journal::{JournalMode, wal_path},
    lock::WriterLock,
    snapshot::{RecordDelta, RecordWindow, Snapshot, read_snapshot, read_snapshot_windows, write_delta, write_snapshot},
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
        let mut writer_lock = WriterLock::acquire(&path)?;
        let enable_wal = journal_mode == JournalMode::Wal;
        let pager = if path.exists() && wal_path(&path).exists() {
            let mut pager = FilePager::open(&path, true)?;
            if !enable_wal {
                writer_lock.ensure_persisted()?;
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

    /// Fence epoch held by this writer handle.
    pub fn writer_fence(&self) -> u64 {
        self._writer_lock.fence()
    }

    /// Switch journal mode, creating or checkpointing sidecars as needed.
    pub fn set_journal_mode(&mut self, mode: JournalMode) -> Result<()> {
        if self.journal_mode == mode {
            return Ok(());
        }
        self.ensure_fence()?;
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

    /// Load metadata plus the records inside `windows`, skipping every other
    /// record.
    ///
    /// This is the read-path counterpart of [`Database::snapshot`]: a narrowed
    /// statement only needs the reserved rows and the record-key window its plan
    /// can read, so the record tree never walks the rest of the database.
    pub fn snapshot_windows(&mut self, windows: &[RecordWindow]) -> Result<Snapshot> {
        read_snapshot_windows(&mut self.pager, windows)
    }

    /// Cumulative publication counters for this database handle.
    ///
    /// Measurement surface: frame counts, durability barriers and time spent in
    /// each publication stage. Reading them never alters publication behavior.
    pub fn publish_stats(&self) -> PublishStats {
        self.pager.publish_stats()
    }

    /// Commit a snapshot, publishing `blob_refs` into the WAL transaction when set.
    pub fn commit(&mut self, snapshot: &Snapshot, blob_refs: &[BlobManifestDelta]) -> Result<()> {
        self.ensure_fence()?;
        self.pager.mutate_with_publish_blob_refs(|inner| write_snapshot(inner, snapshot), blob_refs)
    }

    /// Commit a caller-supplied [`RecordDelta`] instead of scanning for changes.
    ///
    /// The delta carries exactly the records the statement touched, so publishing
    /// never walks the record tree to discover what changed.
    pub fn commit_delta(
        &mut self,
        metadata: &std::collections::BTreeMap<Vec<u8>, Vec<u8>>,
        delta: &RecordDelta,
        blob_refs: &[BlobManifestDelta],
    ) -> Result<()> {
        self.ensure_fence()?;
        self.pager.mutate_with_publish_blob_refs(|inner| write_delta(inner, metadata, delta), blob_refs)
    }

    /// Persist the writer fence epoch before the first publication.
    fn ensure_fence(&mut self) -> Result<()> {
        self._writer_lock.ensure_persisted()
    }

    /// Fold WAL frames into the main file.
    pub fn checkpoint(&mut self) -> Result<()> {
        self.ensure_fence()?;
        self.pager.checkpoint()
    }

    /// Whether the WAL sidecar holds frames not yet folded into the main file.
    pub fn has_pending_wal(&self) -> bool {
        self.pager.has_pending_wal()
    }

    /// Re-run WAL recovery.
    ///
    /// `open` already performs one pass. This entry point makes the report
    /// observable and lets callers prove that a second pass neither truncates
    /// more bytes nor changes the recovered snapshot.
    pub fn recover(&mut self) -> Result<RecoveryReport> {
        self.pager.recover()
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
        Ok(parse_wal_recover(&bytes)?.frames.len() as u32)
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
