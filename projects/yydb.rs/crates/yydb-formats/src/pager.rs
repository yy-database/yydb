//! Page storage trait used by the record B+ tree.

use std::sync::Arc;

use yydb_types::{Error, Result};

use crate::{
    header::DatabaseHeader,
    internal::InternalPage,
    leaf::LeafPage,
    page::{PAGE_TYPE_INTERNAL, PAGE_TYPE_LEAF, PageHeader},
};

/// A page decoded once and reused by read paths.
///
/// A B+ tree lookup otherwise parses the page header twice (once for routing,
/// once in the leaf/internal decoder) and materializes every separator. Pages
/// are immutable between commits, so the decoded form is cached and dropped by
/// the next write to the same page id.
#[derive(Debug, Clone)]
pub enum DecodedPage {
    /// Internal routing page.
    Internal(InternalPage),
    /// Leaf page holding cells.
    Leaf(LeafPage),
    /// Any other page type; read paths reject it.
    Other,
}

impl DecodedPage {
    /// Decode one page image, validating id and checksum via `PageHeader`.
    pub fn parse(page_id: u32, image: &[u8]) -> Result<Self> {
        let header = PageHeader::parse(image)?;
        if header.page_id != page_id {
            return Err(Error::Corrupt("page id mismatch"));
        }
        match header.page_type {
            PAGE_TYPE_LEAF => Ok(Self::Leaf(LeafPage::decode(page_id, image)?)),
            PAGE_TYPE_INTERNAL => Ok(Self::Internal(InternalPage::decode(page_id, image)?)),
            _ => Ok(Self::Other),
        }
    }
}

/// Minimal page map operations for `RecordTree`.
pub trait PageStore {
    /// Load page 0 header.
    fn header(&self) -> Result<DatabaseHeader>;
    /// Read a page image.
    fn get_page(&self, page_id: u32) -> Result<Option<Vec<u8>>>;
    /// Replace a page image.
    fn put_page(&mut self, page_id: u32, image: Vec<u8>) -> Result<()>;
    /// Allocate a new page id.
    fn alloc_page_id(&mut self) -> Result<u32>;
    /// Publish a new record-tree root in page 0.
    fn set_record_root(&mut self, page_id: u32) -> Result<()>;
    /// Return a previously decoded page, when the backend caches one.
    fn decoded_page(&self, _page_id: u32) -> Option<Arc<DecodedPage>> {
        None
    }
    /// Remember a decoded page. Backends without a cache ignore this.
    fn store_decoded_page(&mut self, _page_id: u32, _page: Arc<DecodedPage>) {}
}
