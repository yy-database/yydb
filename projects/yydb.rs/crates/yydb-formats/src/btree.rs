//! B+ tree record store over a page pager.

use std::cmp::Ordering;
use std::sync::Arc;

use yydb_types::{Error, Result};

use crate::{
    internal::{InternalEntry, InternalPage, encoded_entries_bytes},
    key::{TREE_RECORD, TreeKey},
    leaf::{LeafCell, LeafPage},
    page::{PAGE_PAYLOAD_LEN, PAGE_TYPE_INTERNAL, PAGE_TYPE_LEAF, PageHeader},
    pager::{DecodedPage, PageStore},
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

    /// Insert or replace a batch of rows in one tree traversal.
    ///
    /// Rows are sorted once, grouped by the leaf that owns their key range and
    /// merged into that leaf in a single pass. Each touched leaf pays one decode
    /// and one encode per produced page instead of one pair per row, and new
    /// pages are packed to the page budget rather than split at half occupancy.
    /// Duplicate keys keep their last value.
    pub fn put_batch(&mut self, mut rows: Vec<(TreeKey, Vec<u8>)>) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        rows.sort_by(|left, right| left.0.cmp(&right.0));
        let mut entries: Vec<(TreeKey, Vec<u8>)> = Vec::with_capacity(rows.len());
        for (key, value) in rows {
            match entries.last_mut() {
                Some((last_key, last_value)) if *last_key == key => *last_value = value,
                _ => entries.push((key, value)),
            }
        }

        let root = self.pager.header()?.slot.record_root;
        if root == 0 {
            let cells = entries.into_iter().map(|(key, value)| LeafCell { key, value }).collect();
            let groups = pack_leaf_cells(cells)?;
            let mut children = Vec::with_capacity(groups.len());
            for cells in groups {
                let page_id = self.pager.alloc_page_id()?;
                let mut leaf = LeafPage::empty(page_id, 1, TREE_RECORD);
                leaf.set_cells(cells);
                let separator = leaf.cells()[0].key.clone();
                self.pager.put_page(page_id, leaf.encode()?)?;
                children.push(InternalEntry { separator, child_page_id: page_id });
            }
            if children.len() == 1 {
                self.pager.set_record_root(children[0].child_page_id)?;
                return Ok(());
            }
            children[0].separator = TreeKey::min_separator(TREE_RECORD);
            let new_root = self.build_internal_chain(children)?;
            self.pager.set_record_root(new_root)?;
            return Ok(());
        }

        let siblings = self.insert_batch_at(root, entries)?;
        if !siblings.is_empty() {
            let mut entries =
                vec![InternalEntry { separator: TreeKey::min_separator(TREE_RECORD), child_page_id: root }];
            entries.extend(
                siblings.into_iter().map(|(separator, child_page_id)| InternalEntry { separator, child_page_id }),
            );
            let new_root = self.build_internal_chain(entries)?;
            self.pager.set_record_root(new_root)?;
        }
        Ok(())
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
        let page = self.load_decoded(page_id)?.ok_or(Error::Corrupt("missing tree page"))?;
        match page.as_ref() {
            DecodedPage::Leaf(leaf) => Ok(leaf.get(key).map(|v| v.to_vec())),
            DecodedPage::Internal(node) => {
                let child = node.child_page_id(node.child_index_for(key));
                self.get_at(child, key)
            }
            _ => Err(Error::Corrupt("unexpected page type")),
        }
    }

    /// Load a page through the backend's decoded-page cache.
    fn load_decoded(&mut self, page_id: u32) -> Result<Option<Arc<DecodedPage>>> {
        if let Some(page) = self.pager.decoded_page(page_id) {
            return Ok(Some(page));
        }
        let Some(bytes) = self.pager.get_page(page_id)?
        else {
            return Ok(None);
        };
        let page = Arc::new(DecodedPage::parse(page_id, &bytes)?);
        self.pager.store_decoded_page(page_id, page.clone());
        Ok(Some(page))
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

    /// Apply sorted rows to one subtree, returning the right siblings it created.
    fn insert_batch_at(&mut self, page_id: u32, entries: Vec<(TreeKey, Vec<u8>)>) -> Result<Vec<(TreeKey, u32)>> {
        let page = self.load_page(page_id)?.ok_or(Error::Corrupt("missing tree page"))?;
        match page.0.page_type {
            PAGE_TYPE_LEAF => self.apply_leaf_batch(page_id, &page.1, entries),
            PAGE_TYPE_INTERNAL => {
                let node = InternalPage::decode(page_id, &page.1)?;
                let mut groups: Vec<(usize, Vec<(TreeKey, Vec<u8>)>)> = Vec::new();
                for (key, value) in entries {
                    let child_idx = node.child_index_for(&key);
                    match groups.last_mut() {
                        Some((last_idx, rows)) if *last_idx == child_idx => rows.push((key, value)),
                        _ => groups.push((child_idx, vec![(key, value)])),
                    }
                }
                let mut rebuilt = Vec::with_capacity(node.entries.len());
                let mut cursor = 0usize;
                for (child_idx, rows) in groups {
                    if child_idx >= node.entries.len() {
                        return Err(Error::Corrupt("internal child index out of range"));
                    }
                    for entry in &node.entries[cursor..=child_idx] {
                        rebuilt.push(entry.clone());
                    }
                    cursor = child_idx + 1;
                    let siblings = self.insert_batch_at(node.entries[child_idx].child_page_id, rows)?;
                    rebuilt.extend(siblings.into_iter().map(|(separator, child_page_id)| InternalEntry {
                        separator,
                        child_page_id,
                    }));
                }
                rebuilt.extend(node.entries[cursor..].iter().cloned());

                let groups = pack_internal_entries(rebuilt)?;
                if groups.len() == 1 {
                    let entries = groups.into_iter().next().unwrap();
                    let updated = InternalPage {
                        page_id,
                        page_generation: node.page_generation,
                        tree_id: node.tree_id,
                        entries,
                    };
                    self.pager.put_page(page_id, updated.encode()?)?;
                    return Ok(Vec::new());
                }
                let mut siblings = Vec::with_capacity(groups.len() - 1);
                for (index, entries) in groups.into_iter().enumerate() {
                    let separator = entries[0].separator.clone();
                    if index == 0 {
                        let updated = InternalPage {
                            page_id,
                            page_generation: node.page_generation,
                            tree_id: node.tree_id,
                            entries,
                        };
                        self.pager.put_page(page_id, updated.encode()?)?;
                    }
                    else {
                        let right_id = self.pager.alloc_page_id()?;
                        let right = InternalPage {
                            page_id: right_id,
                            page_generation: node.page_generation,
                            tree_id: node.tree_id,
                            entries,
                        };
                        self.pager.put_page(right_id, right.encode()?)?;
                        siblings.push((separator, right_id));
                    }
                }
                Ok(siblings)
            }
            _ => Err(Error::Corrupt("unexpected page type")),
        }
    }

    /// Merge rows into one leaf and pack it into as many pages as the budget needs.
    fn apply_leaf_batch(
        &mut self,
        page_id: u32,
        image: &[u8],
        entries: Vec<(TreeKey, Vec<u8>)>,
    ) -> Result<Vec<(TreeKey, u32)>> {
        let mut leaf = LeafPage::decode(page_id, image)?;
        let cells = merge_cells(leaf.take_cells(), entries);
        let groups = pack_leaf_cells(cells)?;
        let mut siblings = Vec::with_capacity(groups.len() - 1);
        for (index, cells) in groups.into_iter().enumerate() {
            let separator = cells[0].key.clone();
            if index == 0 {
                let mut first = LeafPage::empty(page_id, leaf.page_generation, leaf.tree_id);
                first.set_cells(cells);
                self.pager.put_page(page_id, first.encode()?)?;
            }
            else {
                let right_id = self.pager.alloc_page_id()?;
                let mut right = LeafPage::empty(right_id, leaf.page_generation, leaf.tree_id);
                right.set_cells(cells);
                self.pager.put_page(right_id, right.encode()?)?;
                siblings.push((separator, right_id));
            }
        }
        Ok(siblings)
    }

    /// Build one or more internal levels above a sorted child list and return
    /// the root page id.
    fn build_internal_chain(&mut self, entries: Vec<InternalEntry>) -> Result<u32> {
        let mut level = entries;
        loop {
            let groups = pack_internal_entries(level)?;
            if groups.len() == 1 {
                let page_id = self.pager.alloc_page_id()?;
                let page = InternalPage::new(page_id, 1, TREE_RECORD, groups.into_iter().next().unwrap())?;
                self.pager.put_page(page_id, page.encode()?)?;
                return Ok(page_id);
            }
            let mut next = Vec::with_capacity(groups.len());
            for group in groups {
                let separator = group[0].separator.clone();
                let page_id = self.pager.alloc_page_id()?;
                let page = InternalPage::new(page_id, 1, TREE_RECORD, group)?;
                self.pager.put_page(page_id, page.encode()?)?;
                next.push(InternalEntry { separator, child_page_id: page_id });
            }
            next[0].separator = TreeKey::min_separator(TREE_RECORD);
            level = next;
        }
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
        let page = self.load_decoded(page_id)?.ok_or(Error::Corrupt("missing tree page"))?;
        match page.as_ref() {
            DecodedPage::Leaf(leaf) => {
                for cell in leaf.cells() {
                    out.push((cell.key.clone(), cell.value.clone()));
                }
            }
            DecodedPage::Internal(node) => {
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
        let page = self.load_decoded(page_id)?.ok_or(Error::Corrupt("missing tree page"))?;
        match page.as_ref() {
            DecodedPage::Leaf(leaf) => {
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
            DecodedPage::Internal(node) => {
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

/// Merge sorted existing cells with sorted incoming rows, replacing duplicates.
fn merge_cells(existing: Vec<LeafCell>, entries: Vec<(TreeKey, Vec<u8>)>) -> Vec<LeafCell> {
    let mut merged = Vec::with_capacity(existing.len() + entries.len());
    let mut left = 0usize;
    let mut right = 0usize;
    while left < existing.len() && right < entries.len() {
        match existing[left].key.cmp(&entries[right].0) {
            Ordering::Less => {
                merged.push(LeafCell { key: existing[left].key.clone(), value: existing[left].value.clone() });
                left += 1;
            }
            Ordering::Greater => {
                merged.push(LeafCell { key: entries[right].0.clone(), value: entries[right].1.clone() });
                right += 1;
            }
            Ordering::Equal => {
                merged.push(LeafCell { key: existing[left].key.clone(), value: entries[right].1.clone() });
                left += 1;
                right += 1;
            }
        }
    }
    while left < existing.len() {
        merged.push(LeafCell { key: existing[left].key.clone(), value: existing[left].value.clone() });
        left += 1;
    }
    while right < entries.len() {
        merged.push(LeafCell { key: entries[right].0.clone(), value: entries[right].1.clone() });
        right += 1;
    }
    merged
}

/// Pack cells into as few leaf pages as the payload budget allows.
fn pack_leaf_cells(cells: Vec<LeafCell>) -> Result<Vec<Vec<LeafCell>>> {
    if cells.is_empty() {
        return Err(Error::Corrupt("leaf page needs at least one cell"));
    }
    let mut groups: Vec<Vec<LeafCell>> = Vec::new();
    let mut current: Vec<LeafCell> = Vec::new();
    let mut usage = 4usize;
    for cell in cells {
        if 4 + 4 + LeafPage::cell_bytes_len(None, &cell) > PAGE_PAYLOAD_LEN {
            return Err(Error::Unsupported("record is too large for one leaf page"));
        }
        let mut slot_len = 4 + LeafPage::cell_bytes_len(current.last().map(|cell| &cell.key), &cell);
        if !current.is_empty() && usage + slot_len > PAGE_PAYLOAD_LEN {
            groups.push(std::mem::take(&mut current));
            usage = 4;
            slot_len = 4 + LeafPage::cell_bytes_len(None, &cell);
        }
        usage += slot_len;
        current.push(cell);
    }
    groups.push(current);
    Ok(groups)
}

/// Pack internal entries into pages keeping every page at two or more children.
fn pack_internal_entries(entries: Vec<InternalEntry>) -> Result<Vec<Vec<InternalEntry>>> {
    if entries.len() < 2 {
        return Err(Error::Corrupt("internal page needs two children"));
    }
    let mut groups: Vec<Vec<InternalEntry>> = Vec::new();
    let mut current: Vec<InternalEntry> = Vec::new();
    let mut usage = 2usize;
    for entry in entries {
        let entry_len = 2 + 4 + entry.separator.to_bytes().len();
        if 2 + entry_len > PAGE_PAYLOAD_LEN {
            return Err(Error::Unsupported("separator is too large for one internal page"));
        }
        if !current.is_empty() && usage + entry_len > PAGE_PAYLOAD_LEN {
            groups.push(std::mem::take(&mut current));
            usage = 2;
        }
        usage += entry_len;
        current.push(entry);
    }
    groups.push(current);

    // A trailing page with a single child cannot encode; merge it with the
    // previous page and split that pair at a byte-balanced point.
    if groups.len() > 1 && groups.last().is_some_and(|last| last.len() < 2) {
        let tail = groups.pop().unwrap();
        let mut previous = groups.pop().unwrap();
        previous.extend(tail);
        let (left, right) = split_internal_entries(previous)?;
        groups.push(left);
        groups.push(right);
    }
    Ok(groups)
}

/// Split an entry list into two encodable, byte-balanced halves.
fn split_internal_entries(mut entries: Vec<InternalEntry>) -> Result<(Vec<InternalEntry>, Vec<InternalEntry>)> {
    if entries.len() < 4 {
        return Err(Error::Corrupt("internal page too small to split"));
    }
    let mut best: Option<(usize, usize)> = None;
    for split_at in 2..entries.len() - 1 {
        let left = encoded_entries_bytes(&entries[..split_at]);
        let right = encoded_entries_bytes(&entries[split_at..]);
        if left > PAGE_PAYLOAD_LEN || right > PAGE_PAYLOAD_LEN {
            continue;
        }
        let imbalance = left.abs_diff(right);
        if best.is_none_or(|(_, current)| imbalance <= current) {
            best = Some((split_at, imbalance));
        }
    }
    let Some((split_at, _)) = best
    else {
        return Err(Error::Corrupt("internal page cannot split into two encodable halves"));
    };
    let right = entries.split_off(split_at);
    Ok((entries, right))
}
