//! YYDB format v1 parsing, validation, and in-memory page backend.

#![warn(missing_docs)]

pub mod blob;
pub mod btree;
pub mod crc32c;
pub mod header;
pub mod internal;
pub mod key;
pub mod leaf;
pub mod memory;
pub mod page;
pub mod shm;
pub mod wal;

pub use blob::{parse_blob_header, BlobChunkHeader, BLOB_HEADER_BYTES, BLOB_MAGIC};
pub use btree::RecordTree;
pub use crc32c::crc32c;
pub use header::{
    encode_empty_page0, parse_page0, DatabaseHeader, DatabaseHeaderSlot, PAGE_MAGIC, PAGE_SIZE,
    SLOT_BYTES,
};
pub use internal::{InternalEntry, InternalPage};
pub use key::{TreeKey, KEY_KIND_USER, TREE_CATALOG, TREE_RECORD};
pub use leaf::{LeafCell, LeafPage};
pub use memory::MemoryPager;
pub use page::{encode_page, PageHeader, PAGE_HEADER_LEN, PAGE_PAYLOAD_LEN, PAGE_TYPE_INTERNAL, PAGE_TYPE_LEAF};
pub use shm::{encode_empty_shm, parse_shm, ShmBlock, SHM_BYTES, SHM_MAGIC};
pub use wal::{
    committed_transactions, parse_header, parse_wal, WalFile, WalFrame, WalHeader, WAL_MAGIC,
};
