//! B+ tree record store over a page pager (single-leaf root in v1 slice 1).

use yydb_types::{Error, Result};

use crate::key::{TreeKey, TREE_RECORD};
use crate::leaf::LeafPage;
use crate::memory::MemoryPager;
use crate::page::{PageHeader, PAGE_TYPE_LEAF};

/// KV operations on the record tree inside a pager.
pub struct RecordTree<'a> {
    pager: &'a mut MemoryPager,
}

impl<'a> RecordTree<'a> {
    /// Bind to a pager's record tree.
    pub fn open(pager: &'a mut MemoryPager) -> Self {
        Self { pager }
    }

    /// Point lookup.
    pub fn get(&mut self, key: &TreeKey) -> Result<Option<Vec<u8>>> {
        let root = self.pager.header()?.slot.record_root;
        if root == 0 {
            return Ok(None);
        }
        let page = self
            .pager
            .get_page(root)?
            .ok_or(Error::Corrupt("missing record root page"))?;
        let header = PageHeader::parse(&page)?;
        if header.page_type != PAGE_TYPE_LEAF {
            return Err(Error::Unsupported("record tree internal nodes"));
        }
        Ok(LeafPage::decode(root, &page)?.get(key).map(|v| v.to_vec()))
    }

    /// Insert or replace a value.
    pub fn put(&mut self, key: TreeKey, value: Vec<u8>) -> Result<()> {
        let root = self.pager.header()?.slot.record_root;
        if root == 0 {
            let page_id = self.pager.alloc_page_id()?;
            let mut leaf = LeafPage::empty(page_id, 1, TREE_RECORD);
            if !leaf.upsert(key, value)? {
                return Err(Error::Corrupt("leaf page full"));
            }
            self.pager.put_page(page_id, leaf.encode()?)?;
            self.pager.set_record_root(page_id)?;
            return Ok(());
        }
        let page = self
            .pager
            .get_page(root)?
            .ok_or(Error::Corrupt("missing record root page"))?
            .clone();
        let header = PageHeader::parse(&page)?;
        if header.page_type != PAGE_TYPE_LEAF {
            return Err(Error::Unsupported("record tree internal nodes"));
        }
        let mut leaf = LeafPage::decode(root, &page)?;
        if !leaf.upsert(key, value)? {
            return Err(Error::Corrupt("leaf page full"));
        }
        self.pager.put_page(root, leaf.encode()?)?;
        Ok(())
    }

    /// Delete a key when present.
    pub fn delete(&mut self, key: &TreeKey) -> Result<bool> {
        let root = self.pager.header()?.slot.record_root;
        if root == 0 {
            return Ok(false);
        }
        let page = self
            .pager
            .get_page(root)?
            .ok_or(Error::Corrupt("missing record root page"))?
            .clone();
        let header = PageHeader::parse(&page)?;
        if header.page_type != PAGE_TYPE_LEAF {
            return Err(Error::Unsupported("record tree internal nodes"));
        }
        let mut leaf = LeafPage::decode(root, &page)?;
        if leaf.delete(key) {
            self.pager.put_page(root, leaf.encode()?)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
