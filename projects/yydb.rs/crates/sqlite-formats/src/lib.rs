//! SQLite format 3 binary primitives (pure Rust, no file I/O or SQL execution).
//!
//! This crate mirrors the split in `yydb-formats`: it owns only the on-disk
//! format facts — header, record encoding, blank database materialization —
//! and re-exports them without any storage-layer behaviour.

mod format;
mod record;

pub use crate::{
    format::{
        ENGINE_LIBRARY_VERSION, ENGINE_LIBRARY_VERSION_NUMBER, MAGIC, blank_database, blank_with_page_size,
        bump_change_counter, decode_library_version, read_change_counter, read_freelist_count, read_freelist_trunk,
        read_page_count, read_schema_cookie, set_freelist_count, set_freelist_trunk, set_page_count, set_schema_cookie,
        stamp_library_version, validate_database, validate_header,
    },
    record::{
        RecordLimits, RecordValue, TextEncoding, WriteValue, decode_record, decode_varint, encode_record, encode_varint_into,
    },
};

/// SQLite format reader library version string.
pub fn sqlite_library_version() -> String {
    ENGINE_LIBRARY_VERSION.to_string()
}
