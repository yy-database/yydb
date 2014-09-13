//! Page storage trait used by the record B+ tree.

use yydb_types::Result;

use crate::header::DatabaseHeader;

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
}
