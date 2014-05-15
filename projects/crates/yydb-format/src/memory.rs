//! In-memory page map backend for format v1 engine development.

use std::collections::BTreeMap;

use yydb_types::{Error, Result};

use crate::header::{encode_empty_page0, parse_page0, DatabaseHeader, PAGE_SIZE, SLOT_BYTES};
use crate::key::TreeKey;
use crate::pager::PageStore;
use crate::{btree::RecordTree, crc32c::crc32c};

/// Process-local page store used by tests and the memory storage profile.
#[derive(Debug, Clone, Default)]
pub struct MemoryPager {
    pages: BTreeMap<u32, Vec<u8>>,
}

impl MemoryPager {
    /// Create an empty pager with only page 0 header installed.
    pub fn new_empty(database_id: [u8; 16], storage_layout: u8) -> Self {
        let page0 = encode_empty_page0(database_id, storage_layout);
        let mut pager = Self::default();
        pager.pages.insert(0, page0);
        pager
    }

    /// Load page 0 and return the active database header.
    pub fn header(&self) -> Result<DatabaseHeader> {
        let page0 = self.get_page(0)?.ok_or(Error::Corrupt("missing page0"))?;
        parse_page0(&page0)
    }

    /// Read a page by id.
    pub fn get_page(&self, page_id: u32) -> Result<Option<Vec<u8>>> {
        Ok(self.pages.get(&page_id).cloned())
    }

    /// Write or replace a full page image.
    pub fn put_page(&mut self, page_id: u32, image: Vec<u8>) -> Result<()> {
        if image.len() != PAGE_SIZE {
            return Err(Error::Corrupt("page image wrong size"));
        }
        self.pages.insert(page_id, image);
        Ok(())
    }

    /// Page count including page 0.
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Allocate the next unused page id above the current max.
    pub fn alloc_page_id(&mut self) -> Result<u32> {
        let max_id = self.pages.keys().max().copied().unwrap_or(0);
        Ok(max_id + 1)
    }

    /// Snapshot of all resident pages.
    pub fn pages_snapshot(&self) -> BTreeMap<u32, Vec<u8>> {
        self.pages.clone()
    }

    /// Update `record_root` in both header slots on page 0.
    pub fn set_record_root(&mut self, page_id: u32) -> Result<()> {
        let page0 = self
            .get_page(0)?
            .ok_or(Error::Corrupt("missing page0"))?
            .clone();
        let mut updated = page0;
        for offset in [0, SLOT_BYTES] {
            updated[offset + 48..offset + 52].copy_from_slice(&page_id.to_le_bytes());
            let checksum = crc32c(&updated[offset..offset + 2044]);
            updated[offset + 2044..offset + 2048].copy_from_slice(&checksum.to_le_bytes());
        }
        self.put_page(0, updated)?;
        Ok(())
    }

    /// User KV put on the record tree.
    pub fn put_kv(&mut self, key: impl AsRef<[u8]>, value: &[u8]) -> Result<()> {
        RecordTree::open(self).put(TreeKey::user_record(key), value.to_vec())
    }

    /// User KV get from the record tree.
    pub fn get_kv(&mut self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        RecordTree::open(self).get(&TreeKey::user_record(key))
    }

    /// User KV delete from the record tree.
    pub fn delete_kv(&mut self, key: impl AsRef<[u8]>) -> Result<bool> {
        RecordTree::open(self).delete(&TreeKey::user_record(key))
    }
}

impl PageStore for MemoryPager {
    fn header(&self) -> Result<DatabaseHeader> {
        MemoryPager::header(self)
    }

    fn get_page(&self, page_id: u32) -> Result<Option<Vec<u8>>> {
        MemoryPager::get_page(self, page_id)
    }

    fn put_page(&mut self, page_id: u32, image: Vec<u8>) -> Result<()> {
        MemoryPager::put_page(self, page_id, image)
    }

    fn alloc_page_id(&mut self) -> Result<u32> {
        MemoryPager::alloc_page_id(self)
    }

    fn set_record_root(&mut self, page_id: u32) -> Result<()> {
        MemoryPager::set_record_root(self, page_id)
    }
}
