//! In-memory page map backend for format v1 engine development.

use std::collections::BTreeMap;

use yydb_types::{Error, Result};

use crate::header::{encode_empty_page0, DatabaseHeader, PAGE_SIZE};

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
        let page0 = self
            .get_page(0)?
            .ok_or(Error::Corrupt("missing page0"))?;
        crate::header::parse_page0(&page0)
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
}
