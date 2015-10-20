//! In-memory page map backend for format v0 engine development.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap},
    fs::File,
    io::{Read, Seek, SeekFrom},
    sync::{Arc, Mutex},
};

use yydb_types::{Error, Result};

use crate::{
    btree::RecordTree,
    header::{
        DatabaseHeader, PAGE_SIZE, SLOT_BYTES, SLOT_KIND_A, SLOT_KIND_B, encode_empty_page0, inactive_slot_offset,
        parse_page0_checked, seal_header_slot, slot_offset_for_kind,
    },
    key::TreeKey,
    pager::{DecodedPage, PageStore},
};

/// Lazy reader for a `YDPG` main file.
///
/// Opening a database attaches this source instead of copying every page image
/// into the resident map. A page read on demand stays resident for later
/// lookups, so a reopen pays for the pages the workload actually visits.
#[derive(Debug)]
pub struct MainFileSource {
    page_count: u32,
    /// Shared seek handle so a read does not reopen the file per page.
    file: Mutex<File>,
}

impl MainFileSource {
    /// Bind a source to an already opened main file and its page bound.
    pub fn new(file: File, page_count: u32) -> Self {
        Self { page_count, file: Mutex::new(file) }
    }

    /// Number of pages the main file holds.
    pub fn page_count(&self) -> u32 {
        self.page_count
    }

    fn read_page(&self, page_id: u32) -> Result<Option<Vec<u8>>> {
        if page_id >= self.page_count {
            return Ok(None);
        }
        let mut file = self.file.lock().map_err(|_| Error::Corrupt("main file handle poisoned"))?;
        file.seek(SeekFrom::Start(u64::from(page_id) * PAGE_SIZE as u64))?;
        let mut image = vec![0_u8; PAGE_SIZE];
        file.read_exact(&mut image)?;
        Ok(Some(image))
    }
}

/// Bound on lazily read page images kept in memory (4 KiB each).
const LAZY_PAGE_CACHE: usize = 1024;

/// Process-local page store used by tests and the memory storage profile.
#[derive(Debug, Clone, Default)]
pub struct MemoryPager {
    pages: BTreeMap<u32, Vec<u8>>,
    /// Main file that backs pages which are not resident yet.
    source: Option<Arc<MainFileSource>>,
    /// Bounded read cache for pages pulled from the main file.
    lazy_pages: RefCell<HashMap<u32, Vec<u8>>>,
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
        let header = parse_page0_checked(&page0)?;
        *self.header_cache.borrow_mut() = Some(header.clone());
        Ok(header)
    }

    /// Back pages that are not resident with a lazily read main file.
    pub fn attach_source(&mut self, source: Arc<MainFileSource>) {
        self.source = Some(source);
    }

    /// Read a page by id, pulling it from the main file when it is not resident.
    ///
    /// Pages read from the main file land in a bounded cache so a scan or a
    /// burst of point gets does not re-read the same image from disk.
    pub fn get_page(&self, page_id: u32) -> Result<Option<Vec<u8>>> {
        if let Some(image) = self.pages.get(&page_id) {
            return Ok(Some(image.clone()));
        }
        if let Some(image) = self.lazy_pages.borrow().get(&page_id) {
            return Ok(Some(image.clone()));
        }
        let Some(source) = &self.source else {
            return Ok(None);
        };
        let Some(image) = source.read_page(page_id)? else {
            return Ok(None);
        };
        let mut cache = self.lazy_pages.borrow_mut();
        if cache.len() < LAZY_PAGE_CACHE {
            cache.insert(page_id, image.clone());
        }
        Ok(Some(image))
    }

    /// Write or replace a full page image.
    pub fn put_page(&mut self, page_id: u32, image: Vec<u8>) -> Result<()> {
        if image.len() != PAGE_SIZE {
            return Err(Error::Corrupt("page image wrong size"));
        }
        self.pages.insert(page_id, image);
        self.dirty.insert(page_id);
        self.lazy_pages.borrow_mut().remove(&page_id);
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

    /// Page bound including page 0, covering both resident and main-file pages.
    pub fn page_count(&self) -> usize {
        let resident = self.pages.keys().next_back().map(|id| *id as usize + 1).unwrap_or(0);
        resident.max(self.source.as_ref().map(|source| source.page_count as usize).unwrap_or(0))
    }

    /// Allocate the next unused page id above the current max.
    pub fn alloc_page_id(&mut self) -> Result<u32> {
        let max_resident = self.pages.keys().next_back().copied().unwrap_or(0);
        let max_source = self.source.as_ref().map(|source| source.page_count.saturating_sub(1)).unwrap_or(0);
        Ok(max_resident.max(max_source) + 1)
    }

    /// Snapshot of every page, materialising lazily read main-file pages.
    pub fn pages_snapshot(&self) -> Result<BTreeMap<u32, Vec<u8>>> {
        let mut pages = self.pages.clone();
        for (page_id, image) in self.lazy_pages.borrow().iter() {
            pages.entry(*page_id).or_insert_with(|| image.clone());
        }
        if let Some(source) = &self.source {
            for page_id in 0..source.page_count {
                if !pages.contains_key(&page_id) {
                    if let Some(image) = source.read_page(page_id)? {
                        pages.insert(page_id, image);
                    }
                }
            }
        }
        Ok(pages)
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

    /// Collect the decoded leaf pages holding user keys in `[start, end)`.
    ///
    /// Returns reference-counted handles to the pages already decoded for this
    /// pager, so a read-only caller decodes rows out of those images instead of
    /// copying every stored cell into a fresh map.
    pub fn leaf_pages_range(
        &mut self,
        start: impl AsRef<[u8]>,
        end: Option<&[u8]>,
    ) -> Result<Vec<Arc<DecodedPage>>> {
        let end = end.map(TreeKey::user_record);
        let mut pages = Vec::new();
        RecordTree::open(self).collect_leaf_pages(&TreeKey::user_record(start), end.as_ref(), &mut pages)?;
        Ok(pages)
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
