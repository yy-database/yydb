//! In-memory page map backend for format v0 engine development.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};

use yydb_types::{Error, Result};

use crate::{
    btree::RecordTree,
    header::{
        DatabaseHeader, PAGE_SIZE, SLOT_BYTES, SLOT_KIND_A, SLOT_KIND_B, encode_empty_page0, inactive_slot_offset, parse_page0,
        seal_header_slot, slot_offset_for_kind,
    },
    key::TreeKey,
    pager::{DecodedPage, PageStore},
};

/// Process-local page store used by tests and the memory storage profile.
#[derive(Debug, Clone, Default)]
pub struct MemoryPager {
    pages: BTreeMap<u32, Vec<u8>>,
    /// Pages written since the last [`Self::clear_dirty`]. A publish transaction
    /// brackets its mutation with `clear_dirty` / `take_dirty_pages` so the
    /// changed-page set comes from the writes themselves instead of a full
    /// before/after page-map comparison.
    dirty: BTreeSet<u32>,
    /// Active header parsed from page 0, refreshed whenever page 0 is written.
    header_cache: RefCell<Option<DatabaseHeader>>,
    /// Pages decoded by read paths, dropped when the same page id is rewritten.
    decoded: RefCell<HashMap<u32, Arc<DecodedPage>>>,
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
        if let Some(header) = self.header_cache.borrow().as_ref() {
            return Ok(header.clone());
        }
        let page0 = self.get_page(0)?.ok_or(Error::Corrupt("missing page0"))?;
        let header = parse_page0(&page0)?;
        *self.header_cache.borrow_mut() = Some(header.clone());
        Ok(header)
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
        self.dirty.insert(page_id);
        self.decoded.borrow_mut().remove(&page_id);
        if page_id == 0 {
            *self.header_cache.borrow_mut() = None;
        }
        Ok(())
    }

    /// Decoded page cached by the read paths, if present.
    pub fn decoded_page(&self, page_id: u32) -> Option<Arc<DecodedPage>> {
        self.decoded.borrow().get(&page_id).cloned()
    }

    /// Remember a decoded page. The cache is reset wholesale at a fixed bound
    /// rather than maintained as an LRU; decoded pages are small relative to
    /// the page map they shadow.
    pub fn store_decoded_page(&mut self, page_id: u32, page: Arc<DecodedPage>) {
        let mut decoded = self.decoded.borrow_mut();
        if decoded.len() >= 8_192 {
            decoded.clear();
        }
        decoded.insert(page_id, page);
    }

    /// Forget the dirty set without consuming it, used when a transaction is
    /// abandoned before publishing.
    pub fn clear_dirty(&mut self) {
        self.dirty.clear();
    }

    /// Drain the pages written since the last `clear_dirty` in ascending id
    /// order. Pages removed from the map while dirty are skipped, which matches
    /// the old full-diff behaviour where a vanished page was never published.
    pub fn take_dirty_pages(&mut self) -> Vec<(u32, Vec<u8>)> {
        let dirty = std::mem::take(&mut self.dirty);
        dirty.into_iter().filter_map(|page_id| self.pages.get(&page_id).map(|image| (page_id, image.clone()))).collect()
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
        let inactive_kind = if inactive_offset == 0 { SLOT_KIND_A } else { SLOT_KIND_B };
        let next_generation = header.slot.generation.saturating_add(1);
        let page0 = self.get_page(0)?.ok_or(Error::Corrupt("missing page0"))?.clone();
        let mut updated = page0;
        let active_slot = updated[active_offset..active_offset + SLOT_BYTES].to_vec();
        updated[inactive_offset..inactive_offset + SLOT_BYTES].copy_from_slice(&active_slot);
        updated[inactive_offset + 5] = inactive_kind;
        updated[inactive_offset + 6..inactive_offset + 14].copy_from_slice(&next_generation.to_le_bytes());
        updated[inactive_offset + 14..inactive_offset + 22].copy_from_slice(&checkpoint_lsn.to_le_bytes());
        seal_header_slot(&mut updated, inactive_offset)?;
        self.put_page(0, updated)?;
        Ok(())
    }

    /// Update `record_root` in the active header slot on page 0.
    pub fn set_record_root(&mut self, page_id: u32) -> Result<()> {
        let header = self.header()?;
        let active_offset = slot_offset_for_kind(header.slot.slot_kind);
        let page0 = self.get_page(0)?.ok_or(Error::Corrupt("missing page0"))?.clone();
        let mut updated = page0;
        updated[active_offset + 48..active_offset + 52].copy_from_slice(&page_id.to_le_bytes());
        seal_header_slot(&mut updated, active_offset)?;
        self.put_page(0, updated)?;
        Ok(())
    }

    /// User KV put on the record tree.
    pub fn put_kv(&mut self, key: impl AsRef<[u8]>, value: &[u8]) -> Result<()> {
        RecordTree::open(self).put(TreeKey::user_record(key), value.to_vec())
    }

    /// Insert or replace several user KV rows in one record-tree traversal.
    pub fn put_kv_batch(&mut self, rows: Vec<(Vec<u8>, Vec<u8>)>) -> Result<()> {
        let rows = rows.into_iter().map(|(key, value)| (TreeKey::user_record(key), value)).collect();
        RecordTree::open(self).put_batch(rows)
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

    /// Enumerate record-tree cells whose user key lies in `[start, end)`.
    pub fn scan_kv_range(&mut self, start: impl AsRef<[u8]>, end: Option<&[u8]>) -> Result<Vec<(TreeKey, Vec<u8>)>> {
        let end = end.map(TreeKey::user_record);
        RecordTree::open(self).scan_range(&TreeKey::user_record(start), end.as_ref())
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

    fn decoded_page(&self, page_id: u32) -> Option<Arc<DecodedPage>> {
        MemoryPager::decoded_page(self, page_id)
    }

    fn store_decoded_page(&mut self, page_id: u32, page: Arc<DecodedPage>) {
        MemoryPager::store_decoded_page(self, page_id, page);
    }
}
