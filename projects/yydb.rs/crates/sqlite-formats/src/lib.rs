//! SQLite format 3 binary primitives (pure Rust, no file I/O or SQL execution).
//!
//! This crate mirrors the split in `yydb-formats`: it owns only the on-disk
//! format facts — header, record encoding, blank database materialization —
//! and re-exports them without any storage-layer behaviour.

mod format;
mod record;

pub use crate::{
    format::{
        ENGINE_LIBRARY_VERSION, ENGINE_LIBRARY_VERSION_NUMBER, MAGIC, blank_database, decode_library_version,
        stamp_library_version, validate_database, validate_header,
    },
    record::{RecordLimits, RecordValue, TextEncoding, decode_record, decode_varint},
};

/// SQLite format reader library version string.
pub fn sqlite_library_version() -> String {
    ENGINE_LIBRARY_VERSION.to_string()
}
