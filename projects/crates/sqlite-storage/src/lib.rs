//! SQLite read-only storage runtime.
//!
//! This crate sits above `sqlite-formats`: format primitives for header,
//! record, and encoding come from there. Everything that needs to open a file,
//! traverse a B-tree, or build a complete row payload lives here.

mod database;
mod schema;
mod table;

pub use crate::{
    database::{SqliteDatabase, read_existing},
    schema::{SchemaObject, SchemaObjectKind, read_named_table, read_schema},
    table::{TableLimits, TableRow, read_table},
};

// Re-exported from sqlite-formats for convenient one-crate consumption.
pub use sqlite_formats::{TextEncoding, decode_record, decode_varint, sqlite_library_version};
