//! YYDB format v0 parsing, validation, and in-memory page backend.

#![deny(missing_docs)]

pub mod blob;
pub mod btree;
pub mod crc32c;
pub mod doctor;
pub mod file;
pub mod header;
pub mod internal;
pub mod key;
pub mod leaf;
pub mod memory;
pub mod page;
pub mod pager;
pub mod shm;
pub mod wal;
pub mod wal_append;

pub use blob::{
    blob_chunk_hash, blob_payload_hash, encode_blob_chunk, parse_blob_chunk, parse_blob_header,
    read_blob_payload, BlobChunkHeader, BLOB_CHUNK_DOMAIN, BLOB_CHUNK_KIND_DATA, BLOB_HEADER_BYTES,
    BLOB_MAGIC,
};
pub use btree::RecordTree;
pub use crc32c::crc32c;
pub use doctor::diagnose_file;
pub use file::FilePager;
pub use file::{main_bytes_from_memory_pager, memory_pager_from_main_bytes};
pub use header::{
    encode_empty_page0, parse_page0, DatabaseHeader, DatabaseHeaderSlot, PAGE_MAGIC, PAGE_SIZE,
    SLOT_BYTES,
};
pub use internal::{InternalEntry, InternalPage};
pub use key::{TreeKey, KEY_KIND_USER, TREE_CATALOG, TREE_RECORD};
pub use leaf::{LeafCell, LeafPage};
pub use memory::MemoryPager;
pub use page::{
    encode_page, PageHeader, PAGE_HEADER_LEN, PAGE_PAYLOAD_LEN, PAGE_TYPE_INTERNAL, PAGE_TYPE_LEAF,
};
pub use pager::PageStore;
pub use shm::{
    encode_empty_shm, encode_shm, parse_shm, shm_sidecar_path, sync_shm_from_wal, ShmBlock,
    SHM_BYTES, SHM_MAGIC,
};
pub use wal::{
    committed_tail_lsn, committed_transactions, parse_header, parse_wal, parse_wal_recover,
    wal_frame_offsets, WalFile, WalFrame, WalHeader, WAL_MAGIC,
};
pub use wal_append::strip_trailing_txn_commit;
pub use wal_append::{read_wal_file, replay_wal_pages, wal_sidecar_path, WalWriter};
