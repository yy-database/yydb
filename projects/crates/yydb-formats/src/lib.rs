//! YYDB format v0 parsing, validation, and in-memory page backend.

#![deny(missing_docs)]

pub mod blob;
pub mod blob_refs;
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
    BLOB_CHUNK_DOMAIN, BLOB_CHUNK_KIND_DATA, BLOB_HEADER_BYTES, BLOB_MAGIC, BlobChunkHeader, blob_chunk_hash,
    blob_payload_hash, encode_blob_chunk, parse_blob_chunk, parse_blob_header, read_blob_payload,
};
pub use blob_refs::{
    BLOB_REF_OP_DELETE, BLOB_REF_OP_PUT, BlobChunkRef, BlobManifestDelta, FRAME_BLOB_REFS, canonical_blob_refs_bytes,
    commit_digest, encode_blob_refs_body,
};
pub use btree::RecordTree;
pub use crc32c::crc32c;
pub use doctor::diagnose_file;
pub use file::{FilePager, main_bytes_from_memory_pager, memory_pager_from_main_bytes};
pub use header::{
    DatabaseHeader, DatabaseHeaderSlot, PAGE_MAGIC, PAGE_SIZE, SLOT_BYTES, SLOT_KIND_A, SLOT_KIND_B, encode_empty_page0,
    inactive_slot_offset, parse_page0, slot_offset_for_kind,
};
pub use internal::{InternalEntry, InternalPage};
pub use key::{KEY_KIND_USER, TREE_CATALOG, TREE_RECORD, TreeKey};
pub use leaf::{LeafCell, LeafPage};
pub use memory::MemoryPager;
pub use page::{PAGE_HEADER_LEN, PAGE_PAYLOAD_LEN, PAGE_TYPE_INTERNAL, PAGE_TYPE_LEAF, PageHeader, encode_page};
pub use pager::PageStore;
pub use shm::{SHM_BYTES, SHM_MAGIC, ShmBlock, encode_empty_shm, encode_shm, parse_shm, shm_sidecar_path, sync_shm_from_wal};
pub use wal::{
    MAX_WAL_RECORD_BYTES, WAL_HEADER_BYTES, WAL_MAGIC, WAL_RECORD_HEADER_BYTES, WAL_RECORD_TRAILER_BYTES, WalFile, WalFrame,
    WalHeader, WalRecordError, WalScan, WalTail, committed_tail_lsn, committed_transactions, parse_frame, parse_header,
    parse_wal, parse_wal_recover, scan_wal, wal_frame_offsets,
};
pub use wal_append::{
    WalWriter, encode_frame, encode_wal_header, read_wal_file, replay_wal_pages, strip_incomplete_txn_after_last_page_image,
    strip_trailing_txn_commit, truncate_wal_after_frame, truncate_wal_to_recoverable_prefix, wal_recoverable_byte_len,
    wal_sidecar_path,
};
