//! Journal modes and `{path}-wal` / `{path}-shm` sidecar helpers.
//!
//! Durable journal coordination lives in `yydb-storage`; this module keeps the
//! engine-facing path stable.

pub use yydb_storage::journal::{JournalMode, OpenFlags, shm_path, sidecar_status, wal_path};
