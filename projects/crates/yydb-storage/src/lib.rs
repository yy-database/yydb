//! Durable YYDB file access: main-file lifecycle, writer locking, snapshot
//! commit, WAL / checkpoint coordination, durable CAS blob I/O, and storage
//! metrics.
//!
//! This crate knows the physical `YDPG` / `YYWL` / `YYBB` v0 layout. It does not
//! know VOS, query planning, UDFs, or `Connection`, and it never receives
//! engine state: callers hand over opaque metadata rows plus logical records.

#![deny(missing_docs)]

pub mod blob;
pub mod files;
pub mod journal;
pub mod snapshot;

mod database;
mod lock;

pub use blob::BlobStore;
pub use database::{Database, file_is_ydpg, snapshot_from_main_bytes, snapshot_to_main_bytes};
pub use journal::{JournalMode, OpenFlags, shm_path, wal_path};
pub use lock::{LockMode, WriterLock};
pub use snapshot::{PagerKv, Snapshot};

// Physical layout vocabulary that upper layers need while staying one hop away
// from `yydb-formats`: CAS chunk hashes and WAL blob-manifest deltas are part of
// durable storage, not query or engine semantics.
pub use yydb_formats::{BLOB_REF_OP_DELETE, BLOB_REF_OP_PUT, BlobChunkRef, BlobManifestDelta, blob_chunk_hash, diagnose_file};
