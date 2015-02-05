//! SQLite product facade.
//!
//! Two distinct capabilities share this crate:
//!
//! | Capability | Entry point | Storage |
//! |------------|-------------|---------|
//! | SQLite **dialect** over YYDB | [`SqliteConnection`] | `.yydb` file, YYDB engine and operators |
//! | Read-only **file import** | [`SqliteDatabase`] | existing upstream `.sqlite` bytes, parsed in place |
//!
//! [`SqliteConnection`] executes SQL with YYDB's own engine: `oak-sql` parses,
//! `yydb-query` lowers the AST into the same internal operators VOS uses, and
//! `yydb-engine` supplies transactions, catalog and storage. No SQLite C
//! library or C sys-binding is involved.
//!
//! [`SqliteDatabase`] only reads an existing SQLite file so its rows can be
//! adopted into a YYDB database; it never becomes the execution engine.
//!
//! Schema creation stays VOS-owned: both surfaces use a VOS schema document as
//! DDL, and SQL statements cover rows (see [`SqliteConnection::ensure_schema`]).

#![deny(missing_docs)]

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

mod connection;

pub use connection::SqliteConnection;

/// Adapter identifier used in logs and diagnostics.
pub const BACKEND_ID: &str = "sqlite";

/// Short label for the binary-compatibility constraint.
pub const CONSTRAINT: &str = "upstream SQLite format-3 engine";

/// Why YYDB executes SQL itself instead of calling the SQLite engine.
pub const WHY_NOT_YY_OPTIMIZED: &str = "SQL execution uses YYDB operators. The SQLite engine is only a read-only file source.";
