//! B+ tree record store over a page pager.

use yydb_types::{Error, Result};

use crate::internal::{InternalEntry, InternalPage};
use crate::key::{TreeKey, TREE_RECORD};
use crate::leaf::LeafPage;
use crate::pager::PageStore;
use crate::page::{PageHeader, PAGE_TYPE_INTERNAL, PAGE_TYPE_LEAF};

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
                        InternalEntry {
                            separator: TreeKey::min_separator(TREE_RECORD),
                            child_page_id: root,
                        },
                        InternalEntry {
                            separator,
                            child_page_id: right_id,
                        },
                    ],
                )?;
                self.pager.put_page(internal_id, internal.encode()?)?;
                self.pager.set_record_root(internal_id)?;
                Ok(())
            }
        }
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
        let page = self
            .load_page(page_id)?
            .ok_or(Error::Corrupt("missing tree page"))?;
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

    fn insert_at(
        &mut self,
        page_id: u32,
        key: TreeKey,
        value: Vec<u8>,
    ) -> Result<Option<(TreeKey, u32)>> {
        let page = self
            .load_page(page_id)?
            .ok_or(Error::Corrupt("missing tree page"))?;
        match page.0.page_type {
            PAGE_TYPE_LEAF => {
                let leaf = self.insert_leaf(page_id, key, value)?;
                Ok(leaf.split.map(|(separator, right_id)| (separator, right_id)))
            }
            PAGE_TYPE_INTERNAL => {
                let node = InternalPage::decode(page_id, &page.1)?;
                let child_idx = node.child_index_for(&key);
                let child_id = node.child_page_id(child_idx);
                if let Some((separator, right_id)) = self.insert_at(child_id, key, value)? {
                    let mut internal = node;
                    if internal.insert_child(child_idx, separator, right_id)? {
                        self.pager.put_page(page_id, internal.encode()?)?;
                        return Ok(None);
                    }
                    let (promoted, mut right_internal) = internal.split()?;
                    let right_id = self.pager.alloc_page_id()?;
                    right_internal.page_id = right_id;
                    self.pager.put_page(page_id, internal.encode()?)?;
                    self.pager.put_page(right_id, right_internal.encode()?)?;
                    return Ok(Some((promoted, right_id)));
                }
                Ok(None)
            }
            _ => Err(Error::Corrupt("unexpected page type")),
        }
    }

    fn insert_leaf(&mut self, page_id: u32, key: TreeKey, value: Vec<u8>) -> Result<LeafInsert> {
        let mut leaf = match self.load_page(page_id)? {
            Some((header, bytes)) if header.page_type == PAGE_TYPE_LEAF => {
                LeafPage::decode(page_id, &bytes)?
            }
            Some(_) => return Err(Error::Corrupt("expected leaf page")),
            None => LeafPage::empty(page_id, 1, TREE_RECORD),
        };
        if leaf.upsert(key.clone(), value.clone())? {
            self.pager.put_page(page_id, leaf.encode()?)?;
            return Ok(LeafInsert {
                page_id,
                split: None,
            });
        }
        let (separator, mut right) = leaf.split()?;
        let right_id = self.pager.alloc_page_id()?;
        right.page_id = right_id;
        if key >= separator {
            if !right.upsert(key, value)? {
                return Err(Error::Corrupt("right leaf full after split"));
            }
        } else if !leaf.upsert(key, value)? {
            return Err(Error::Corrupt("left leaf full after split"));
        }
        self.pager.put_page(page_id, leaf.encode()?)?;
        self.pager.put_page(right_id, right.encode()?)?;
        Ok(LeafInsert {
            page_id,
            split: Some((separator, right_id)),
        })
    }

    fn delete_at(&mut self, page_id: u32, key: &TreeKey) -> Result<bool> {
        let page = self
            .load_page(page_id)?
            .ok_or(Error::Corrupt("missing tree page"))?;
        match page.0.page_type {
            PAGE_TYPE_LEAF => {
                let mut leaf = LeafPage::decode(page_id, &page.1)?;
                if leaf.delete(key) {
                    self.pager.put_page(page_id, leaf.encode()?)?;
                    Ok(true)
                } else {
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

    fn load_page(&mut self, page_id: u32) -> Result<Option<(PageHeader, Vec<u8>)>> {
        let page = self.pager.get_page(page_id)?;
        let Some(bytes) = page else {
            return Ok(None);
        };
        Ok(Some((PageHeader::parse(&bytes)?, bytes)))
    }
}
