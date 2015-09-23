//! B+ tree record store over a page pager.

use yydb_types::{Error, Result};

use crate::{
    internal::{InternalEntry, InternalPage},
    key::{TREE_RECORD, TreeKey},
    leaf::{LeafCell, LeafPage},
    page::{PAGE_TYPE_INTERNAL, PAGE_TYPE_LEAF, PageHeader},
    pager::PageStore,
};

/// Result of inserting into a leaf that may have split.
struct LeafInsert {
    page_id: u32,
    split: Option<(TreeKey, u32)>,
}

/// KV operations on the record tree inside a pager.
pub struct RecordTree<'a, P: PageStore + ?Sized> {
    pager: &'a mut P,
}

impl<'a, P: PageStore + ?Sized> RecordTree<'a, P> {
    /// Bind to a pager's record tree.
    pub fn open(pager: &'a mut P) -> Self {
        Self { pager }
    }

    /// Point lookup.
    pub fn get(&mut self, key: &TreeKey) -> Result<Option<Vec<u8>>> {
        let root = self.pager.header()?.slot.record_root;
        if root == 0 {
            return Ok(None);
        }
        self.get_at(root, key)
    }

    /// Insert or replace a value.
    pub fn put(&mut self, key: TreeKey, value: Vec<u8>) -> Result<()> {
        let root = self.pager.header()?.slot.record_root;
        if root == 0 {
            let page_id = self.pager.alloc_page_id()?;
            let leaf = self.insert_leaf(page_id, key, value)?;
            self.pager.set_record_root(leaf.page_id)?;
            return Ok(());
        }
        match self.insert_at(root, key, value)? {
            None => Ok(()),
            Some((separator, right_id)) => {
                let internal_id = self.pager.alloc_page_id()?;
                let internal = InternalPage::new(
                    internal_id,
                    1,
                    TREE_RECORD,
                    vec![
                        InternalEntry { separator: TreeKey::min_separator(TREE_RECORD), child_page_id: root },
                        InternalEntry { separator, child_page_id: right_id },
                    ],
                )?;
                self.pager.put_page(internal_id, internal.encode()?)?;
                self.pager.set_record_root(internal_id)?;
                Ok(())
            }
        }
    }

    /// Visit every key/value in the record tree.
    pub fn scan_all(&mut self) -> Result<Vec<(TreeKey, Vec<u8>)>> {
        let root = self.pager.header()?.slot.record_root;
        if root == 0 {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        self.scan_page(root, &mut out)?;
        Ok(out)
    }

    /// Visit the keys in `[start, end)` in ascending order.
    ///
    /// The walk descends only into subtrees that can hold keys inside the
    /// window, so a narrow window costs one root-to-leaf path plus the leaves it
    /// actually covers instead of the height-independent full traversal of
    /// [`scan_all`](Self::scan_all). Cell keys are still tested one by one, so
    /// the result matches the same window over a full scan.
    ///
    /// `end` is exclusive and `None` means "to the last key".
    pub fn scan_range(&mut self, start: &TreeKey, end: Option<&TreeKey>) -> Result<Vec<(TreeKey, Vec<u8>)>> {
        let root = self.pager.header()?.slot.record_root;
        if root == 0 {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        self.scan_page_range(root, start, end, &mut out)?;
        Ok(out)
    }

    /// Delete a key when present.
    pub fn delete(&mut self, key: &TreeKey) -> Result<bool> {
        let root = self.pager.header()?.slot.record_root;
        if root == 0 {
            return Ok(false);
        }
        self.delete_at(root, key)
    }

    fn get_at(&mut self, page_id: u32, key: &TreeKey) -> Result<Option<Vec<u8>>> {
        let page = self.load_page(page_id)?.ok_or(Error::Corrupt("missing tree page"))?;
        match page.0.page_type {
            PAGE_TYPE_LEAF => Ok(LeafPage::decode(page_id, &page.1)?.get(key).map(|v| v.to_vec())),
            PAGE_TYPE_INTERNAL => {
                let node = InternalPage::decode(page_id, &page.1)?;
                let child = node.child_page_id(node.child_index_for(key));
                self.get_at(child, key)
            }
            _ => Err(Error::Corrupt("unexpected page type")),
        }
    }

    fn insert_at(&mut self, page_id: u32, key: TreeKey, value: Vec<u8>) -> Result<Option<(TreeKey, u32)>> {
        let page = self.load_page(page_id)?.ok_or(Error::Corrupt("missing tree page"))?;
        match page.0.page_type {
            PAGE_TYPE_LEAF => {
                let leaf = self.insert_leaf(page_id, key, value)?;
                Ok(leaf.split)
            }
            PAGE_TYPE_INTERNAL => {
                let node = InternalPage::decode(page_id, &page.1)?;
                let child_idx = node.child_index_for(&key);
                let child_id = node.child_page_id(child_idx);
                if let Some((separator, right_id)) = self.insert_at(child_id, key, value)? {
                    let mut internal = node;
                    if internal.insert_child(child_idx, separator.clone(), right_id)? {
                        self.pager.put_page(page_id, internal.encode()?)?;
                        return Ok(None);
                    }
                    // The page is full and `insert_child` rolled the new entry
                    // back. Re-insert it before splitting so the split child
                    // stays reachable from exactly one of the two halves;
                    // dropping it here orphans a whole leaf of records.
                    internal.entries.insert(child_idx + 1, InternalEntry { separator, child_page_id: right_id });
                    let (promoted, mut right_internal) = internal.split()?;
                    let split_right_id = self.pager.alloc_page_id()?;
                    right_internal.page_id = split_right_id;
                    self.pager.put_page(page_id, internal.encode()?)?;
                    self.pager.put_page(split_right_id, right_internal.encode()?)?;
                    return Ok(Some((promoted, split_right_id)));
                }
                Ok(None)
            }
            _ => Err(Error::Corrupt("unexpected page type")),
        }
    }

    fn insert_leaf(&mut self, page_id: u32, key: TreeKey, value: Vec<u8>) -> Result<LeafInsert> {
        let mut leaf = match self.load_page(page_id)? {
            Some((header, bytes)) if header.page_type == PAGE_TYPE_LEAF => LeafPage::decode(page_id, &bytes)?,
            Some(_) => return Err(Error::Corrupt("expected leaf page")),
            None => LeafPage::empty(page_id, 1, TREE_RECORD),
        };
        if leaf.upsert(key.clone(), value.clone())? {
            self.pager.put_page(page_id, leaf.encode()?)?;
            return Ok(LeafInsert { page_id, split: None });
        }
        // A replacement that grew past the page keeps its key, so drop the old
        // cell before the size-aware split re-inserts the new value.
        leaf.delete(&key);
        let (separator, mut right) = leaf.split_for_insert(LeafCell { key, value })?;
        let right_id = self.pager.alloc_page_id()?;
        right.page_id = right_id;
        self.pager.put_page(page_id, leaf.encode()?)?;
        self.pager.put_page(right_id, right.encode()?)?;
        Ok(LeafInsert { page_id, split: Some((separator, right_id)) })
    }

    fn delete_at(&mut self, page_id: u32, key: &TreeKey) -> Result<bool> {
        let page = self.load_page(page_id)?.ok_or(Error::Corrupt("missing tree page"))?;
        match page.0.page_type {
            PAGE_TYPE_LEAF => {
                let mut leaf = LeafPage::decode(page_id, &page.1)?;
                if leaf.delete(key) {
                    self.pager.put_page(page_id, leaf.encode()?)?;
                    Ok(true)
                }
                else {
                    Ok(false)
                }
            }
            PAGE_TYPE_INTERNAL => {
                let node = InternalPage::decode(page_id, &page.1)?;
                let child = node.child_page_id(node.child_index_for(key));
                self.delete_at(child, key)
            }
            _ => Err(Error::Corrupt("unexpected page type")),
        }
    }

    fn scan_page(&mut self, page_id: u32, out: &mut Vec<(TreeKey, Vec<u8>)>) -> Result<()> {
        let page = self.load_page(page_id)?.ok_or(Error::Corrupt("missing tree page"))?;
        match page.0.page_type {
            PAGE_TYPE_LEAF => {
                let leaf = LeafPage::decode(page_id, &page.1)?;
                for cell in leaf.cells() {
                    out.push((cell.key.clone(), cell.value.clone()));
                }
            }
            PAGE_TYPE_INTERNAL => {
                let node = InternalPage::decode(page_id, &page.1)?;
                for idx in 0..node.entries.len() {
                    self.scan_page(node.child_page_id(idx), out)?;
                }
            }
            _ => return Err(Error::Corrupt("unexpected page type")),
        }
        Ok(())
    }

    /// Range walk with subtree pruning. Mirrors [`Self::scan_page`] but skips
    /// children that cannot hold a key inside `[start, end)`.
    fn scan_page_range(
        &mut self,
        page_id: u32,
        start: &TreeKey,
        end: Option<&TreeKey>,
        out: &mut Vec<(TreeKey, Vec<u8>)>,
    ) -> Result<()> {
        let page = self.load_page(page_id)?.ok_or(Error::Corrupt("missing tree page"))?;
        match page.0.page_type {
            PAGE_TYPE_LEAF => {
                let leaf = LeafPage::decode(page_id, &page.1)?;
                for cell in leaf.cells() {
                    if cell.key < *start {
                        continue;
                    }
                    if end.is_some_and(|end| cell.key >= *end) {
                        break;
                    }
                    out.push((cell.key.clone(), cell.value.clone()));
                }
            }
            PAGE_TYPE_INTERNAL => {
                let node = InternalPage::decode(page_id, &page.1)?;
                for idx in 0..node.entries.len() {
                    let lower = &node.entries[idx].separator;
                    // Child `idx` only holds keys below its successor's
                    // separator, so it is entirely below `start` when that
                    // separator is not greater than `start`.
                    if node.entries.get(idx + 1).is_some_and(|next| next.separator <= *start) {
                        continue;
                    }
                    if end.is_some_and(|end| *lower >= *end) {
                        break;
                    }
                    self.scan_page_range(node.child_page_id(idx), start, end, out)?;
                }
            }
            _ => return Err(Error::Corrupt("unexpected page type")),
        }
        Ok(())
    }

    fn load_page(&mut self, page_id: u32) -> Result<Option<(PageHeader, Vec<u8>)>> {
        let page = self.pager.get_page(page_id)?;
        let Some(bytes) = page
        else {
            return Ok(None);
        };
        Ok(Some((PageHeader::parse(&bytes)?, bytes)))
    }
}
