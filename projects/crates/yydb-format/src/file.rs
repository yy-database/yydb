//! `YDPG` file-backed pager with optional `YYWL` v0 sidecar.

use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use yydb_types::{Error, Result};

use crate::btree::RecordTree;
use crate::header::{parse_page0, DatabaseHeader, PAGE_MAGIC, PAGE_SIZE};
use crate::key::TreeKey;
use crate::memory::MemoryPager;
use crate::pager::PageStore;
use crate::wal_append::{read_wal_file, replay_wal_pages, wal_sidecar_path, WalWriter};

/// Persistent page store backed by a single `YDPG` main file.
pub struct FilePager {
    path: PathBuf,
    inner: MemoryPager,
    wal_enabled: bool,
    wal: Option<WalWriter>,
}

impl FilePager {
    /// Create a new database file or open an existing `YDPG` main file.
    pub fn open(path: impl AsRef<Path>, enable_wal: bool) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let existed = path.exists();
        let inner = if existed {
            load_main_file(&path)?
        } else {
            MemoryPager::new_empty(fresh_database_id(), 0x01)
        };
        let mut file_pager = Self {
            path: path.clone(),
            inner,
            wal_enabled: enable_wal,
            wal: None,
        };
        if !existed {
            file_pager.persist_all()?;
        }
        if enable_wal {
            file_pager.ensure_wal()?;
            file_pager.replay_wal_if_present()?;
        }
        Ok(file_pager)
    }

    /// Main file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// WAL sidecar path when enabled.
    pub fn wal_path(&self) -> Option<PathBuf> {
        self.wal.as_ref().map(|wal| wal.path().to_path_buf())
    }

    /// Apply record-tree mutations and publish them as one WAL transaction or write batch.
    pub fn mutate_with_publish<F>(&mut self, mutate: F) -> Result<()>
    where
        F: FnOnce(&mut MemoryPager) -> Result<()>,
    {
        let before = self.inner.pages_snapshot();
        mutate(&mut self.inner)?;
        self.publish_delta(&before)
    }

    /// User KV put with durability.
    pub fn put_kv(&mut self, key: impl AsRef<[u8]>, value: &[u8]) -> Result<()> {
        self.mutate_with_publish(|inner| {
            RecordTree::open(inner).put(TreeKey::user_record(key), value.to_vec())
        })
    }

    /// User KV get.
    pub fn get_kv(&mut self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        RecordTree::open(&mut self.inner).get(&TreeKey::user_record(key))
    }

    /// Enumerate every record-tree cell.
    pub fn scan_kv(&mut self) -> Result<Vec<(TreeKey, Vec<u8>)>> {
        RecordTree::open(&mut self.inner).scan_all()
    }

    /// User KV delete.
    pub fn delete_kv(&mut self, key: impl AsRef<[u8]>) -> Result<bool> {
        let before = self.inner.pages_snapshot();
        let deleted = RecordTree::open(&mut self.inner).delete(&TreeKey::user_record(key))?;
        if deleted {
            self.publish_delta(&before)?;
        }
        Ok(deleted)
    }

    /// Fold WAL into the main file and truncate the sidecar.
    pub fn checkpoint(&mut self) -> Result<()> {
        self.persist_all()?;
        if self.wal_enabled {
            let wal_path = wal_sidecar_path(&self.path);
            if wal_path.exists() {
                std::fs::remove_file(wal_path)?;
            }
            self.wal = None;
        }
        Ok(())
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
        self.wal = None;
        self.wal_enabled = false;
        Ok(())
    }

    fn ensure_wal(&mut self) -> Result<()> {
        if self.wal_enabled && self.wal.is_none() {
            let database_id = self.inner.header()?.slot.database_id;
            self.wal = Some(WalWriter::open(wal_sidecar_path(&self.path), database_id)?);
        }
        Ok(())
    }

    fn publish_delta(&mut self, before: &BTreeMap<u32, Vec<u8>>) -> Result<()> {
        let after = self.inner.pages_snapshot();
        let mut changed = Vec::new();
        for (page_id, image) in &after {
            if before.get(page_id) != Some(image) {
                changed.push((*page_id, image.clone()));
            }
        }
        if changed.is_empty() {
            return Ok(());
        }
        if self.wal_enabled {
            self.ensure_wal()?;
            if let Some(wal) = self.wal.as_mut() {
                wal.append_commit(&changed)?;
            }
        } else {
            for (page_id, image) in &changed {
                self.write_page_to_file(*page_id, image)?;
            }
        }
        Ok(())
    }

    fn replay_wal_if_present(&mut self) -> Result<()> {
        let Some(wal_path) = self.wal_path() else {
            return Ok(());
        };
        if !wal_path.exists() {
            return Ok(());
        }
        let wal = read_wal_file(&wal_path)?;
        let mut pages = self.inner.pages_snapshot();
        replay_wal_pages(&wal, &mut pages)?;
        for (page_id, image) in pages {
            self.inner.put_page(page_id, image)?;
        }
        Ok(())
    }

    fn persist_all(&self) -> Result<()> {
        for (page_id, image) in self.inner.pages_snapshot() {
            self.write_page_to_file(page_id, &image)?;
        }
        Ok(())
    }

    fn write_page_to_file(&self, page_id: u32, image: &[u8]) -> Result<()> {
        if image.len() != PAGE_SIZE {
            return Err(Error::Corrupt("page image wrong size"));
        }
        let offset = u64::from(page_id) * u64::try_from(PAGE_SIZE).unwrap_or(u64::MAX);
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&self.path)?;
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
}

/// Load a `MemoryPager` from a contiguous `YDPG` main-file byte image.
pub fn memory_pager_from_main_bytes(bytes: &[u8]) -> Result<MemoryPager> {
    load_main_bytes(bytes)
}

/// Serialize a `MemoryPager` into a contiguous `YDPG` main-file byte image.
pub fn main_bytes_from_memory_pager(pager: &MemoryPager) -> Result<Vec<u8>> {
    let snapshot = pager.pages_snapshot();
    if snapshot.is_empty() {
        return Err(Error::Corrupt("memory pager has no pages"));
    }
    let max_id = snapshot.keys().max().copied().unwrap_or(0);
    let page_count = max_id as usize + 1;
    let mut out = vec![0_u8; page_count * PAGE_SIZE];
    for page_id in 0..=max_id {
        let image = snapshot
            .get(&page_id)
            .ok_or(Error::Corrupt("memory pager page gap"))?;
        let start = page_id as usize * PAGE_SIZE;
        out[start..start + PAGE_SIZE].copy_from_slice(image);
    }
    Ok(out)
}

fn load_main_file(path: &Path) -> Result<MemoryPager> {
    load_main_bytes(&std::fs::read(path)?)
}

fn load_main_bytes(bytes: &[u8]) -> Result<MemoryPager> {
    if bytes.len() < PAGE_SIZE {
        return Err(Error::Corrupt("main file too short"));
    }
    if bytes.get(0..5) != Some(PAGE_MAGIC.as_slice()) {
        return Err(Error::Unsupported("main file is not YDPG format v0"));
    }
    parse_page0(&bytes[0..PAGE_SIZE])?;
    let page_count = bytes.len() / PAGE_SIZE;
    let mut pager = MemoryPager::default();
    for page_id in 0..page_count as u32 {
        let start = page_id as usize * PAGE_SIZE;
        pager.put_page(page_id, bytes[start..start + PAGE_SIZE].to_vec())?;
    }
    Ok(pager)
}

fn fresh_database_id() -> [u8; 16] {
    [
        0xAA, 0xBB, 0xCC, 0xDD, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 1,
    ]
}
