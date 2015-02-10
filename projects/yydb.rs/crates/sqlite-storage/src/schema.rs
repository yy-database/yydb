use yydb_types::{Error, Result};

use sqlite_formats::{RecordLimits, RecordValue, TextEncoding, decode_record, validate_database};

use crate::table::{TableLimits, TableRow, read_table};

/// Object types stored in the SQLite schema table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaObjectKind {
    /// Ordinary, WITHOUT ROWID, or virtual table.
    Table,
    /// Explicit or automatically created index.
    Index,
    /// Stored view definition, not evaluated by this reader.
    View,
    /// Stored trigger definition, not evaluated by this reader.
    Trigger,
}

/// One typed schema record with SQL preserved as opaque source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaObject {
    /// The object type reported by SQLite.
    pub kind: SchemaObjectKind,
    /// Stored object name, without identifier parsing or normalization.
    pub name: String,
    /// Associated table or view name.
    pub table_name: String,
    /// Stored root page, preserving NULL and zero for non-b-tree objects.
    pub root_page: Option<u32>,
    /// Opaque CREATE source, or NULL for an automatic index.
    pub sql: Option<String>,
}

/// Reads the five schema columns without parsing CREATE statements.
/// Empty uninitialized databases have no schema objects.
pub fn read_schema(bytes: &[u8], limits: TableLimits) -> Result<Vec<SchemaObject>> {
    validate_database(bytes)?;
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let rows = read_table(bytes, 1, limits)?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let encoding = match u32::from_be_bytes(bytes[56..60].try_into().unwrap()) {
        1 => TextEncoding::Utf8,
        2 => TextEncoding::Utf16Le,
        3 => TextEncoding::Utf16Be,
        _ => return Err(Error::Corrupt("sqlite nonempty schema has no text encoding")),
    };
    rows.into_iter().map(|row| decode_schema(&row.payload, encoding, limits.max_payload_bytes)).collect()
}

/// Reads an ordinary rowid table by stored name, using ASCII-insensitive matching.
/// Schema aliases refer to the main-file schema, not a connection's TEMP schema.
/// Views, virtual tables, index b-trees and WITHOUT ROWID tables are unsupported.
pub fn read_named_table(bytes: &[u8], name: &str, limits: TableLimits) -> Result<Vec<TableRow>> {
    if name.eq_ignore_ascii_case("sqlite_schema") || name.eq_ignore_ascii_case("sqlite_master") {
        validate_database(bytes)?;
        return if bytes.is_empty() { Ok(Vec::new()) } else { read_table(bytes, 1, limits) };
    }
    let objects = read_schema(bytes, limits)?;
    let mut matching = objects.iter().filter(|object| {
        matches!(object.kind, SchemaObjectKind::Table | SchemaObjectKind::View) && object.name.eq_ignore_ascii_case(name)
    });
    let object = matching.next().ok_or_else(|| Error::Corrupt("sqlite named table not found"))?;
    if matching.next().is_some() {
        return Err(Error::Corrupt("sqlite ambiguous table name"));
    }
    if object.kind != SchemaObjectKind::Table {
        return Err(Error::Unsupported("sqlite view execution"));
    }
    let root = object.root_page.filter(|root| *root != 0).ok_or(Error::Unsupported("sqlite virtual table access"))?;
    if root == 1 {
        return Err(Error::Corrupt("sqlite user table aliases schema root"));
    }
    read_table(bytes, root, limits)
}

fn decode_schema(payload: &[u8], encoding: TextEncoding, max_payload_bytes: usize) -> Result<SchemaObject> {
    let values = decode_record(payload, encoding, RecordLimits { max_columns: 5, max_payload_bytes })?;
    let [kind, name, table_name, root_page, sql]: [RecordValue<'_>; 5] =
        values.try_into().map_err(|_| Error::Corrupt("sqlite schema column count invalid"))?;
    let kind = match text(kind)?.as_str() {
        "table" => SchemaObjectKind::Table,
        "index" => SchemaObjectKind::Index,
        "view" => SchemaObjectKind::View,
        "trigger" => SchemaObjectKind::Trigger,
        _ => return Err(Error::Corrupt("sqlite schema object kind invalid")),
    };
    let name = text(name)?;
    let table_name = text(table_name)?;
    if name.is_empty() || table_name.is_empty() {
        return Err(Error::Corrupt("sqlite schema object name empty"));
    }
    let root_page = match root_page {
        RecordValue::Null => None,
        RecordValue::Integer(value) => {
            Some(u32::try_from(value).map_err(|_| Error::Corrupt("sqlite schema root page invalid"))?)
        }
        _ => return Err(Error::Corrupt("sqlite schema root page is not integer or NULL")),
    };
    if matches!(kind, SchemaObjectKind::View | SchemaObjectKind::Trigger) && root_page.is_some_and(|root| root != 0) {
        return Err(Error::Corrupt("sqlite schema non-b-tree object has root page"));
    }
    let sql = match sql {
        RecordValue::Null => None,
        RecordValue::Text(value) => Some(value),
        _ => return Err(Error::Corrupt("sqlite schema SQL is not text or NULL")),
    };
    Ok(SchemaObject { kind, name, table_name, root_page, sql })
}

fn text(value: RecordValue<'_>) -> Result<String> {
    match value {
        RecordValue::Text(value) => Ok(value),
        _ => Err(Error::Corrupt("sqlite schema field is not text")),
    }
}
