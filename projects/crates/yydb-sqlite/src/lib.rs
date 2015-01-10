//! SQLite binary-compatible format-3 snapshot reader (pure Rust).
//!
//! This crate inspects on-disk SQLite main files without linking any external
//! SQLite engine. Snapshot reads are bounded and do not recover WAL or journal
//! state. Neither path uses YYDB KV storage or embedded engine execution.

mod database;
mod format;
mod record;
mod schema;
mod table;

pub use crate::{
    database::{SqliteDatabase, read_existing},
    format::{
        ENGINE_LIBRARY_VERSION, ENGINE_LIBRARY_VERSION_NUMBER, MAGIC, blank_database, decode_library_version,
        validate_database, validate_header,
    },
    record::{RecordLimits, RecordValue, TextEncoding, decode_record, decode_varint},
    schema::{SchemaObject, SchemaObjectKind, read_named_table, read_schema},
    table::{TableLimits, TableRow, read_table},
};

/// Adapter identifier used in logs and diagnostics.
pub const BACKEND_ID: &str = "sqlite";

/// Short label for the binary-compatibility constraint.
pub const CONSTRAINT: &str = "upstream SQLite format-3 engine";

/// Why YYDB does not execute SQL through the SQLite engine.
pub const WHY_NOT_YY_OPTIMIZED: &str = "SQLite requires binary-compatible on-disk bytes. YYDB stores VOS in `.yydb` instead.";

/// Returns the SQLite format reader library version string.
pub fn sqlite_library_version() -> String {
    ENGINE_LIBRARY_VERSION.to_string()
}
