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
pub use database::{file_is_ydpg, snapshot_from_main_bytes, snapshot_to_main_bytes, Database};
pub use journal::{shm_path, wal_path, JournalMode, OpenFlags};
pub use lock::WriterLock;
pub use snapshot::{PagerKv, Snapshot};
