//! SQLite product facade — re-exports `sqlite-storage`'s public API and the
//! product surface constants that downstream consumers reference.
//!
//! The real implementation lives in `sqlite-formats` (binary format
//! primitives) and `sqlite-storage` (database handle, schema, table scan).
//! This crate is the stable product entry point above both.

pub use sqlite_storage::{
    SchemaObject, SchemaObjectKind, SqliteDatabase, TableLimits, TableRow, TextEncoding, decode_record, decode_varint,
    read_existing, read_named_table, read_schema, read_table, sqlite_library_version,
};

// Legacy surface constants and format functions that historical consumers
// reference through this facade crate.
pub use sqlite_formats::{
    ENGINE_LIBRARY_VERSION, ENGINE_LIBRARY_VERSION_NUMBER, MAGIC, RecordLimits, RecordValue, blank_database,
    decode_library_version, validate_database,
};

/// Adapter identifier used in logs and diagnostics.
pub const BACKEND_ID: &str = "sqlite";

/// Short label for the binary-compatibility constraint.
pub const CONSTRAINT: &str = "upstream SQLite format-3 engine";

/// Why YYDB does not execute SQL through the SQLite engine.
pub const WHY_NOT_YY_OPTIMIZED: &str = "SQLite requires binary-compatible on-disk bytes. YYDB stores VOS in `.yydb` instead.";
