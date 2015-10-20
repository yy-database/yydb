//! `YDPG` file-backed pager with optional `YYWL` v0 sidecar.

use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use yydb_types::{Error, Result};

use crate::{
    blob_refs::BlobManifestDelta,
    btree::RecordTree,
    header::{
        DatabaseHeader, PAGE_MAGIC, PAGE_SIZE, corrupt_header_slot_checksum, inactive_slot_offset, parse_page0_checked,
        slot_offset_for_kind,
    },
    key::TreeKey,
    memory::{MainFileSource, MemoryPager},
    pager::PageStore,
    shm::{shm_sidecar_path, sync_shm_from_commit, sync_shm_from_wal},
    wal::{committed_tail_lsn, committed_transactions},
    wal_append::{
        FRAME_TXN_BEGIN, RecoveryReport, WalWriteStats, WalWriter, read_wal_file, replay_wal_pages_after,
        truncate_wal_records, truncate_wal_torn_tail, wal_sidecar_path,
    },
};

/// Cumulative publication counters for [`FilePager`].
///
/// Measurement surface only. Counters describe the work a publication already
/// performed, so reading them never changes frames, ordering or durability.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PublishStats {
    /// WAL append counters, including one durability barrier per commit.
    pub wal: WalWriteStats,
    /// `-shm` mirror writes performed after commits.
    pub shm_writes: u64,
    /// Time spent writing the `-shm` mirror.
    pub shm_ns: u128,
}

/// Persistent page store backed by a single `YDPG` main file.
pub struct FilePager {
    path: PathBuf,
    inner: MemoryPager,
    wal_enabled: bool,
    wal: Option<WalWriter>,
    last_recovery: RecoveryReport,
    /// WAL scan results the last recovery pass produced, consumed by
    /// [`FilePager::ensure_wal`] so opening scans the sidecar exactly once.
    recovered_wal: Option<RecoveredWal>,
    /// Whether the sidecar holds frames the main file does not yet contain.
    pending_wal: bool,
    shm_writes: u64,
    shm_ns: u128,
}

/// Tail state a recovery pass already derived from the WAL sidecar.
#[derive(Debug, Clone, Copy)]
struct RecoveredWal {
    /// Highest frame LSN in the sidecar.
    tail_lsn: u64,
    /// Transaction id the next append continues from.
    next_txid: u64,
    /// Sidecar length after the torn tail was truncated.
    file_bytes: u64,
}

impl FilePager {
    /// Create a new database file or open an existing `YDPG` main file.
    pub fn open(path: impl AsRef<Path>, enable_wal: bool) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let existed = path.exists();
        let inner = if existed { open_main_file(&path)? } else { MemoryPager::new_empty(fresh_database_id(), 0x01) };
        let mut file_pager = Self {
            path: path.clone(),
            inner,
            wal_enabled: enable_wal,
            wal: None,
            last_recovery: RecoveryReport::default(),
            recovered_wal: None,
            pending_wal: false,
            shm_writes: 0,
            shm_ns: 0,
        };
        if !existed {
            file_pager.persist_all()?;
        }
        if enable_wal {
            // Recovery reads the sidecar once. The append writer opens on the
            // first publication instead, so an open that never writes pays
            // neither the sidecar creation nor the `-shm` mirror.
            file_pager.recover()?;
        }
        Ok(file_pager)
    }

    /// Main file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// WAL sidecar path when enabled.
    ///
    /// The path is reported for every WAL-enabled handle. The file itself only
    /// exists once a publication created it, so callers must not infer its
    /// presence from this value.
    pub fn wal_path(&self) -> Option<PathBuf> {
        (self.wal_enabled || self.wal.is_some()).then(|| wal_sidecar_path(&self.path))
    }

    /// Apply record-tree mutations and publish them as one WAL transaction or write batch.
    pub fn mutate_with_publish<F>(&mut self, mutate: F) -> Result<()>
    where
        F: FnOnce(&mut MemoryPager) -> Result<()>,
    {
        self.mutate_with_publish_blob_refs(mutate, &[])
    }

    /// Like [`mutate_with_publish`] but also records `.yydx` blob manifest deltas in WAL.
    pub fn mutate_with_publish_blob_refs<F>(&mut self, mutate: F, blob_refs: &[BlobManifestDelta]) -> Result<()>
    where
        F: FnOnce(&mut MemoryPager) -> Result<()>,
    {
        self.inner.clear_dirty();
        mutate(&mut self.inner)?;
        let changed = self.inner.take_dirty_pages();
        self.publish_pages(changed, blob_refs)
    }

    /// User KV put with durability.
    pub fn put_kv(&mut self, key: impl AsRef<[u8]>, value: &[u8]) -> Result<()> {
        self.mutate_with_publish(|inner| RecordTree::open(inner).put(TreeKey::user_record(key), value.to_vec()))
    }

    /// Insert or replace several user KV rows and publish them as one transaction.
    pub fn put_kv_batch(&mut self, rows: Vec<(Vec<u8>, Vec<u8>)>) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let rows = rows.into_iter().map(|(key, value)| (TreeKey::user_record(key), value)).collect();
        self.mutate_with_publish(|inner| RecordTree::open(inner).put_batch(rows))
    }

    /// User KV get.
    pub fn get_kv(&mut self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        RecordTree::open(&mut self.inner).get(&TreeKey::user_record(key))
    }

    /// Enumerate every record-tree cell.
    pub fn scan_kv(&mut self) -> Result<Vec<(TreeKey, Vec<u8>)>> {
        RecordTree::open(&mut self.inner).scan_all()
    }

    /// Enumerate record-tree cells whose user key lies in `[start, end)`.
    pub fn scan_kv_range(&mut self, start: impl AsRef<[u8]>, end: Option<&[u8]>) -> Result<Vec<(TreeKey, Vec<u8>)>> {
        let end = end.map(TreeKey::user_record);
        RecordTree::open(&mut self.inner).scan_range(&TreeKey::user_record(start), end.as_ref())
    }

    /// Collect the decoded leaf pages holding user keys in `[start, end)`.
    ///
    /// The returned handles keep the pager's decoded pages alive without copying
    /// a cell, so a read-only caller can decode rows straight out of the page
    /// images it is handed.
    pub fn leaf_pages_range(
        &mut self,
        start: impl AsRef<[u8]>,
        end: Option<&[u8]>,
    ) -> Result<Vec<Arc<crate::pager::DecodedPage>>> {
        let end = end.map(TreeKey::user_record);
        let mut pages = Vec::new();
        RecordTree::open(&mut self.inner).collect_leaf_pages(&TreeKey::user_record(start), end.as_ref(), &mut pages)?;
        Ok(pages)
    }

    /// User KV delete.
    pub fn delete_kv(&mut self, key: impl AsRef<[u8]>) -> Result<bool> {
        self.inner.clear_dirty();
        let deleted = RecordTree::open(&mut self.inner).delete(&TreeKey::user_record(key))?;
        if deleted {
            let changed = self.inner.take_dirty_pages();
            self.publish_pages(changed, &[])?;
        }
        Ok(deleted)
    }

    /// Fold WAL into the main file and truncate the sidecar.
    ///
    /// Order matters: data pages, then the page 0 header with the new checkpoint
    /// LSN, then record truncation. A crash before the header commit leaves the
    /// WAL replayable. A crash after it leaves only records at or below the
    /// checkpoint LSN, which recovery skips.
    pub fn checkpoint(&mut self) -> Result<()> {
        self.commit_checkpoint_pages()?;
        if self.wal_enabled {
            self.wal = None;
            self.recovered_wal = None;
            let wal_path = wal_sidecar_path(&self.path);
            if wal_path.exists() {
                truncate_wal_records(&wal_path)?;
                std::fs::remove_file(&wal_path)?;
            }
            self.pending_wal = false;
            let shm_path = shm_sidecar_path(&self.path);
            if shm_path.exists() {
                std::fs::remove_file(shm_path)?;
            }
        }
        Ok(())
    }

    /// Persist pages and the page 0 checkpoint slot without truncating the WAL.
    fn commit_checkpoint_pages(&mut self) -> Result<u64> {
        let wal_path = wal_sidecar_path(&self.path);
        if self.wal_enabled && wal_path.exists() {
            truncate_wal_torn_tail(&wal_path)?;
            if let Some(wal) = self.wal.as_mut() {
                wal.refresh_file_bytes()?;
            }
        }
        let checkpoint_lsn = self.committed_wal_tail_lsn()?;
        self.persist_data_pages()?;
        self.inner.bump_checkpoint_slot(checkpoint_lsn)?;
        self.persist_page0()?;
        Ok(checkpoint_lsn)
    }

    /// Whether the `-wal` sidecar holds appended frames that are not yet folded
    /// into the main file. Cheap: it reads the writer's own byte counter.
    pub fn has_pending_wal(&self) -> bool {
        self.wal_enabled && self.pending_wal
    }

    /// Enable `YYWL` v0 journaling for subsequent mutations.
    pub fn enable_wal(&mut self) -> Result<()> {
        self.wal_enabled = true;
        self.ensure_wal()?;
        Ok(())
    }

    /// Checkpoint and stop writing WAL frames.
    pub fn disable_wal(&mut self) -> Result<()> {
        if !self.wal_enabled {
            return Ok(());
        }
        self.persist_all()?;
        let wal_path = wal_sidecar_path(&self.path);
        if wal_path.exists() {
            std::fs::remove_file(wal_path)?;
        }
        let shm_path = shm_sidecar_path(&self.path);
        if shm_path.exists() {
            std::fs::remove_file(shm_path)?;
        }
        self.wal = None;
        self.recovered_wal = None;
        self.pending_wal = false;
        self.wal_enabled = false;
        Ok(())
    }

    fn ensure_wal(&mut self) -> Result<()> {
        if self.wal_enabled && self.wal.is_none() {
            let header = self.inner.header()?;
            let wal_path = wal_sidecar_path(&self.path);
            self.wal = Some(match self.recovered_wal.take() {
                // Recovery already scanned and validated this sidecar, and it
                // wrote the `-shm` mirror for the same tail, so the writer
                // continues from those values instead of re-reading the file.
                Some(recovered) => WalWriter::open_recovered(
                    &wal_path,
                    header.slot.database_id,
                    header.slot.checkpoint_lsn,
                    recovered.tail_lsn,
                    recovered.next_txid,
                    recovered.file_bytes,
                )?,
                None => {
                    let writer = WalWriter::open(&wal_path, header.slot.database_id, header.slot.checkpoint_lsn)?;
                    sync_shm_from_wal(&self.path, &wal_path)?;
                    writer
                }
            });
        }
        Ok(())
    }

    fn publish_pages(&mut self, changed: Vec<(u32, Vec<u8>)>, blob_refs: &[BlobManifestDelta]) -> Result<()> {
        if changed.is_empty() && blob_refs.is_empty() {
            return Ok(());
        }
        if self.wal_enabled {
            self.ensure_wal()?;
            if let Some(wal) = self.wal.as_mut() {
                let commit_lsn = wal.append_commit(&changed, blob_refs)?;
                self.pending_wal = true;
                let wal_bytes = wal.file_bytes();
                let shm_started = Instant::now();
                sync_shm_from_commit(&self.path, commit_lsn, wal_bytes)?;
                self.shm_ns += shm_started.elapsed().as_nanos();
                self.shm_writes += 1;
            }
        }
        else {
            for (page_id, image) in &changed {
                self.write_page_to_file(*page_id, image)?;
            }
        }
        Ok(())
    }

    /// Cumulative publication counters since the pager was opened.
    ///
    /// Not part of the storage contract: callers use these numbers to attribute
    /// write cost between frame encoding, the durability barrier and the shm
    /// mirror. They never change publication behavior.
    pub fn publish_stats(&self) -> PublishStats {
        PublishStats {
            wal: self.wal.as_ref().map(WalWriter::write_stats).unwrap_or_default(),
            shm_writes: self.shm_writes,
            shm_ns: self.shm_ns,
        }
    }

    fn committed_wal_tail_lsn(&self) -> Result<u64> {
        let header_lsn = self.inner.header()?.slot.checkpoint_lsn;
        let wal_path = wal_sidecar_path(&self.path);
        if !wal_path.exists() {
            return Ok(header_lsn);
        }
        Ok(committed_tail_lsn(&read_wal_file(&wal_path)?).max(header_lsn))
    }

    /// Run startup recovery: truncate a torn tail, replay complete committed
    /// frames above the checkpoint LSN and publish the sidecar status.
    ///
    /// A checksum, length or LSN failure aborts recovery with a structured
    /// [`crate::WalRecordError`] instead of silently skipping damaged records.
    /// Running this twice yields the same data and drops no further bytes.
    pub fn recover(&mut self) -> Result<RecoveryReport> {
        let wal_path = wal_sidecar_path(&self.path);
        if !wal_path.exists() {
            self.last_recovery = RecoveryReport::default();
            self.recovered_wal = None;
            self.pending_wal = false;
            return Ok(self.last_recovery);
        }
        let dropped_torn_bytes = truncate_wal_torn_tail(&wal_path)? as u64;
        let wal = read_wal_file(&wal_path)?;
        let checkpoint_lsn = self.inner.header()?.slot.checkpoint_lsn;
        let frames_after_checkpoint = wal.frames.iter().filter(|frame| frame.frame_lsn > checkpoint_lsn).count();
        // Replay only the pages named by committed frames above the checkpoint.
        // Pages that never changed since the checkpoint stay in the main file
        // and are read on demand by `MainFileSource`.
        let mut pages = std::collections::BTreeMap::new();
        replay_wal_pages_after(&wal, checkpoint_lsn, &mut pages)?;
        for (page_id, image) in pages {
            self.inner.put_page(page_id, image)?;
        }
        let wal_bytes = std::fs::metadata(&wal_path).map(|meta| meta.len()).unwrap_or(0);
        sync_shm_from_commit(&self.path, committed_tail_lsn(&wal), wal_bytes)?;
        let tail_lsn = wal.frames.iter().map(|frame| frame.frame_lsn).max().unwrap_or(0);
        let next_txid = wal.frames.iter().filter(|frame| frame.frame_type == FRAME_TXN_BEGIN).count() as u64 + 1;
        self.recovered_wal = Some(RecoveredWal { tail_lsn, next_txid, file_bytes: wal_bytes });
        self.pending_wal = !wal.frames.is_empty();
        self.last_recovery = RecoveryReport {
            dropped_torn_bytes,
            frames: wal.frames.len(),
            frames_after_checkpoint,
            committed_transactions: committed_transactions(&wal),
            committed_lsn: committed_tail_lsn(&wal).max(checkpoint_lsn),
        };
        Ok(self.last_recovery)
    }

    /// Report from the most recent recovery pass.
    pub fn last_recovery(&self) -> RecoveryReport {
        self.last_recovery
    }

    fn persist_all(&self) -> Result<()> {
        self.persist_data_pages()?;
        self.persist_page0()?;
        Ok(())
    }

    fn persist_data_pages(&self) -> Result<()> {
        let pages: Vec<(u32, Vec<u8>)> =
            self.inner.pages_snapshot()?.into_iter().filter(|(page_id, _)| *page_id != 0).collect();
        if pages.is_empty() {
            return Ok(());
        }
        // One handle, one durability barrier. Opening, seeking and syncing per
        // page turned a checkpoint into one fsync per page, which is the
        // difference between a few milliseconds and several seconds.
        let mut file = OpenOptions::new().create(true).read(true).write(true).truncate(false).open(&self.path)?;
        let last_page = pages.iter().map(|(page_id, _)| *page_id).max().unwrap_or(0);
        let needed = (u64::from(last_page) + 1) * PAGE_SIZE as u64;
        if file.metadata()?.len() < needed {
            file.set_len(needed)?;
        }
        for (page_id, image) in &pages {
            if image.len() != PAGE_SIZE {
                return Err(Error::Corrupt("page image wrong size"));
            }
            file.seek(SeekFrom::Start(u64::from(*page_id) * PAGE_SIZE as u64))?;
            file.write_all(image)?;
        }
        file.sync_all()?;
        Ok(())
    }

    fn persist_page0(&self) -> Result<()> {
        let page0 = self.inner.get_page(0)?.ok_or(Error::Corrupt("missing page0"))?;
        self.write_page_to_file(0, &page0)?;
        Ok(())
    }

    /// Persist data pages and bump the in-memory header without writing page 0.
    #[doc(hidden)]
    pub fn simulate_checkpoint_crash_before_header_commit(&mut self) -> Result<()> {
        let checkpoint_lsn = self.committed_wal_tail_lsn()?;
        self.persist_data_pages()?;
        self.inner.bump_checkpoint_slot(checkpoint_lsn)?;
        Ok(())
    }

    /// Simulate a crash after the checkpoint header commit but before WAL truncation.
    #[doc(hidden)]
    pub fn simulate_checkpoint_crash_after_header_commit(&mut self) -> Result<()> {
        self.commit_checkpoint_pages()?;
        Ok(())
    }

    /// Write page 0 with a torn inactive header slot checksum.
    #[doc(hidden)]
    pub fn simulate_checkpoint_crash_corrupt_inactive_header(&mut self) -> Result<()> {
        let checkpoint_lsn = self.committed_wal_tail_lsn()?;
        let header_before = self.inner.header()?;
        let inactive_offset = inactive_slot_offset(slot_offset_for_kind(header_before.slot.slot_kind));
        self.persist_data_pages()?;
        self.inner.bump_checkpoint_slot(checkpoint_lsn)?;
        let page0 = self.inner.get_page(0)?.ok_or(Error::Corrupt("missing page0"))?.clone();
        let mut image = page0;
        corrupt_header_slot_checksum(&mut image, inactive_offset)?;
        self.write_page_to_file(0, &image)?;
        Ok(())
    }

    fn write_page_to_file(&self, page_id: u32, image: &[u8]) -> Result<()> {
        if image.len() != PAGE_SIZE {
            return Err(Error::Corrupt("page image wrong size"));
        }
        let offset = u64::from(page_id) * u64::try_from(PAGE_SIZE).unwrap_or(u64::MAX);
        let mut file = OpenOptions::new().create(true).read(true).write(true).truncate(false).open(&self.path)?;
        let needed = offset + PAGE_SIZE as u64;
        if file.metadata()?.len() < needed {
            file.set_len(needed)?;
        }
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(image)?;
        file.sync_all()?;
        Ok(())
    }
}

impl PageStore for FilePager {
    fn header(&self) -> Result<DatabaseHeader> {
        self.inner.header()
    }

    fn get_page(&self, page_id: u32) -> Result<Option<Vec<u8>>> {
        self.inner.get_page(page_id)
    }

    fn put_page(&mut self, page_id: u32, image: Vec<u8>) -> Result<()> {
        self.inner.put_page(page_id, image)
    }

    fn alloc_page_id(&mut self) -> Result<u32> {
        self.inner.alloc_page_id()
    }

    fn set_record_root(&mut self, page_id: u32) -> Result<()> {
        self.inner.set_record_root(page_id)
    }

    fn decoded_page(&self, page_id: u32) -> Option<std::sync::Arc<crate::pager::DecodedPage>> {
        self.inner.decoded_page(page_id)
    }

    fn store_decoded_page(&mut self, page_id: u32, page: std::sync::Arc<crate::pager::DecodedPage>) {
        self.inner.store_decoded_page(page_id, page);
    }
}

/// Load a `MemoryPager` from a contiguous `YDPG` main-file byte image.
pub fn memory_pager_from_main_bytes(bytes: &[u8]) -> Result<MemoryPager> {
    load_main_bytes(bytes)
}

/// Serialize a `MemoryPager` into a contiguous `YDPG` main-file byte image.
pub fn main_bytes_from_memory_pager(pager: &MemoryPager) -> Result<Vec<u8>> {
    let snapshot = pager.pages_snapshot()?;
    if snapshot.is_empty() {
        return Err(Error::Corrupt("memory pager has no pages"));
    }
    let max_id = snapshot.keys().max().copied().unwrap_or(0);
    let page_count = max_id as usize + 1;
    let mut out = vec![0_u8; page_count * PAGE_SIZE];
    for page_id in 0..=max_id {
        let image = snapshot.get(&page_id).ok_or(Error::Corrupt("memory pager page gap"))?;
        let start = page_id as usize * PAGE_SIZE;
        out[start..start + PAGE_SIZE].copy_from_slice(image);
    }
    Ok(out)
}

/// Open a `YDPG` main file and keep its remaining pages readable on demand.
///
/// Only page 0 is read up front, so a reopen does not copy every page into the
/// resident map. Reads pull the page they touch through [`MainFileSource`].
fn open_main_file(path: &Path) -> Result<MemoryPager> {
    let file = File::open(path)?;
    let len = file.metadata()?.len();
    if len < PAGE_SIZE as u64 {
        return Err(Error::Corrupt("main file too short"));
    }
    if len % PAGE_SIZE as u64 != 0 {
        return Err(Error::Corrupt("main file length is not page aligned"));
    }
    let page_count = (len / PAGE_SIZE as u64) as u32;
    let mut page0 = vec![0_u8; PAGE_SIZE];
    (&file).read_exact(&mut page0)?;
    if page0.get(0..5) != Some(PAGE_MAGIC.as_slice()) {
        return Err(Error::Unsupported("main file is not YDPG format v0"));
    }
    parse_page0_checked(&page0)?;
    let mut pager = MemoryPager::default();
    pager.put_page(0, page0)?;
    pager.attach_source(Arc::new(MainFileSource::new(file, page_count)));
    Ok(pager)
}

fn load_main_bytes(bytes: &[u8]) -> Result<MemoryPager> {
    if bytes.len() < PAGE_SIZE {
        return Err(Error::Corrupt("main file too short"));
    }
    if bytes.get(0..5) != Some(PAGE_MAGIC.as_slice()) {
        return Err(Error::Unsupported("main file is not YDPG format v0"));
    }
    parse_page0_checked(&bytes[0..PAGE_SIZE])?;
    let page_count = bytes.len() / PAGE_SIZE;
    let mut pager = MemoryPager::default();
    for page_id in 0..page_count as u32 {
        let start = page_id as usize * PAGE_SIZE;
        pager.put_page(page_id, bytes[start..start + PAGE_SIZE].to_vec())?;
    }
    Ok(pager)
}

fn fresh_database_id() -> [u8; 16] {
    [0xAA, 0xBB, 0xCC, 0xDD, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 1]
}
