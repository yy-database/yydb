//! In-memory page map backend for format v0 engine development.

use std::collections::BTreeMap;

use yydb_types::{Error, Result};

use crate::header::{
    encode_empty_page0, inactive_slot_offset, parse_page0, seal_header_slot,
    slot_offset_for_kind, DatabaseHeader, PAGE_SIZE, SLOT_BYTES, SLOT_KIND_A, SLOT_KIND_B,
};
use crate::key::TreeKey;
use crate::pager::PageStore;
use crate::btree::RecordTree;

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

    /// Bump `generation` and `checkpoint_lsn` by writing the inactive header slot only.
    pub fn bump_checkpoint_slot(&mut self, checkpoint_lsn: u64) -> Result<()> {
        let header = self.header()?;
        let active_offset = slot_offset_for_kind(header.slot.slot_kind);
        let inactive_offset = inactive_slot_offset(active_offset);
        let inactive_kind = if inactive_offset == 0 {
            SLOT_KIND_A
        } else {
            SLOT_KIND_B
        };
        let next_generation = header.slot.generation.saturating_add(1);
        let page0 = self
            .get_page(0)?
            .ok_or(Error::Corrupt("missing page0"))?
            .clone();
        let mut updated = page0;
        let active_slot = updated[active_offset..active_offset + SLOT_BYTES].to_vec();
        updated[inactive_offset..inactive_offset + SLOT_BYTES].copy_from_slice(&active_slot);
        updated[inactive_offset + 5] = inactive_kind;
        updated[inactive_offset + 6..inactive_offset + 14]
            .copy_from_slice(&next_generation.to_le_bytes());
        updated[inactive_offset + 14..inactive_offset + 22]
            .copy_from_slice(&checkpoint_lsn.to_le_bytes());
        seal_header_slot(&mut updated, inactive_offset)?;
        self.put_page(0, updated)?;
        Ok(())
    }

    /// Update `record_root` in the active header slot on page 0.
    pub fn set_record_root(&mut self, page_id: u32) -> Result<()> {
        let header = self.header()?;
        let active_offset = slot_offset_for_kind(header.slot.slot_kind);
        let page0 = self
            .get_page(0)?
            .ok_or(Error::Corrupt("missing page0"))?
            .clone();
        let mut updated = page0;
        updated[active_offset + 48..active_offset + 52]
            .copy_from_slice(&page_id.to_le_bytes());
        seal_header_slot(&mut updated, active_offset)?;
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

    /// Enumerate every record-tree cell.
    pub fn scan_kv(&mut self) -> Result<Vec<(TreeKey, Vec<u8>)>> {
        RecordTree::open(self).scan_all()
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
