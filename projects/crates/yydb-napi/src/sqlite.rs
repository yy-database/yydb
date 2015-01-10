#![allow(non_snake_case)]

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;
use yydb_sqlite::{
    RecordLimits, RecordValue, SchemaObjectKind, TableLimits, TextEncoding, blank_database, decode_library_version,
    decode_record, read_existing, read_named_table, read_schema, validate_database,
};

/// Optional resource bounds for each schema or table scan.
#[napi(object)]
pub struct SqliteReadLimits {
    pub max_pages: Option<u32>,
    pub max_rows: Option<u32>,
    pub max_payload_bytes: Option<u32>,
    pub max_total_payload_bytes: Option<u32>,
}

impl SqliteReadLimits {
    fn resolve(limits: Option<Self>) -> TableLimits {
        let default = TableLimits::default();
        match limits {
            None => default,
            Some(limits) => TableLimits {
                max_pages: limits.max_pages.map_or(default.max_pages, |v| v as usize),
                max_rows: limits.max_rows.map_or(default.max_rows, |v| v as usize),
                max_payload_bytes: limits.max_payload_bytes.map_or(default.max_payload_bytes, |v| v as usize),
                max_total_payload_bytes: limits.max_total_payload_bytes.map_or(default.max_total_payload_bytes, |v| v as usize),
            },
        }
    }
}

/// The five SQLite schema columns, with SQL retained as opaque source.
#[napi(object)]
pub struct SqliteSchemaObject {
    pub kind: String,
    pub name: String,
    pub table_name: String,
    pub root_page: Option<u32>,
    pub sql: Option<String>,
}

/// An undecoded SQLite table record with lossless rowid and binary payload.
#[napi(object)]
pub struct SqliteTableRow {
    pub rowid: i64,
    pub payload: Buffer,
}

/// One typed, decoded SQLite record field.
#[napi(object)]
pub struct SqliteRecordValue {
    pub kind: String,
    pub value_integer: Option<i64>,
    pub value_real: Option<f64>,
    pub value_text: Option<String>,
    pub value_blob: Option<Buffer>,
}

/// A decoded SQLite row: rowid plus typed column values.
#[napi(object)]
pub struct SqliteDecodedRow {
    pub rowid: i64,
    pub columns: Vec<SqliteRecordValue>,
}

fn encode_encoding(name: Option<&str>) -> napi::Result<TextEncoding> {
    match name {
        None | Some("utf8") | Some("utf-8") | Some("UTF-8") => Ok(TextEncoding::Utf8),
        Some("utf16le") | Some("utf-16le") | Some("UTF-16LE") => Ok(TextEncoding::Utf16Le),
        Some("utf16be") | Some("utf-16be") | Some("UTF-16BE") => Ok(TextEncoding::Utf16Be),
        Some(other) => Err(napi::Error::from_reason(format!(
            "unsupported SQLite text encoding {other}; expected utf8, utf16le, or utf16be"
        ))),
    }
}

fn detect_encoding(bytes: &[u8]) -> napi::Result<TextEncoding> {
    if bytes.len() < 64 {
        return Err(napi::Error::from_reason("sqlite snapshot has no text encoding"));
    }
    match u32::from_be_bytes(bytes[56..60].try_into().unwrap()) {
        1 => Ok(TextEncoding::Utf8),
        2 => Ok(TextEncoding::Utf16Le),
        3 => Ok(TextEncoding::Utf16Be),
        _ => Err(napi::Error::from_reason("sqlite snapshot has unknown text encoding")),
    }
}

fn record_value(value: RecordValue<'_>) -> SqliteRecordValue {
    match value {
        RecordValue::Null => {
            SqliteRecordValue { kind: "null".into(), value_integer: None, value_real: None, value_text: None, value_blob: None }
        }
        RecordValue::Integer(int) => SqliteRecordValue {
            kind: "integer".into(),
            value_integer: Some(i64::from(int)),
            value_real: None,
            value_text: None,
            value_blob: None,
        },
        RecordValue::Real(real) => SqliteRecordValue {
            kind: "real".into(),
            value_integer: None,
            value_real: Some(real),
            value_text: None,
            value_blob: None,
        },
        RecordValue::Text(text) => SqliteRecordValue {
            kind: "text".into(),
            value_integer: None,
            value_real: None,
            value_text: Some(text),
            value_blob: None,
        },
        RecordValue::Blob(blob) => SqliteRecordValue {
            kind: "blob".into(),
            value_integer: None,
            value_real: None,
            value_text: None,
            value_blob: Some(blob.to_vec().into()),
        },
    }
}

fn resolve_encoding_name(bytes: &[u8]) -> napi::Result<&'static str> {
    if bytes.len() < 64 {
        return Ok("utf8");
    }
    Ok(match u32::from_be_bytes(bytes[56..60].try_into().unwrap()) {
        1 => "utf8",
        2 => "utf16le",
        3 => "utf16be",
        _ => return Err(napi::Error::from_reason("sqlite snapshot has unknown text encoding")),
    })
}

fn build_decoded_rows(
    bytes: &[u8],
    rows: Vec<yydb_sqlite::TableRow>,
    limits: TableLimits,
) -> napi::Result<Vec<SqliteDecodedRow>> {
    let encoding = detect_encoding(bytes)?;
    let record_limits = RecordLimits { max_columns: 1024, max_payload_bytes: limits.max_payload_bytes.max(1_048_576) };
    rows.into_iter()
        .map(|row| {
            decode_record(&row.payload, encoding, record_limits).map_err(|e| napi::Error::from_reason(e.to_string())).map(
                |columns| SqliteDecodedRow {
                    rowid: i64::from(row.rowid),
                    columns: columns.into_iter().map(record_value).collect(),
                },
            )
        })
        .collect()
}

/// Owned, read-only main-file bytes, independent of the YYDB engine runtime.
#[napi]
pub struct SqliteSnapshot {
    bytes: Vec<u8>,
}

#[napi]
impl SqliteSnapshot {
    /// Copies a validated snapshot, bounded to 256 MiB unless overridden.
    #[napi(constructor)]
    pub fn new(bytes: Buffer, max_snapshot_bytes: Option<u32>) -> napi::Result<Self> {
        let maximum = max_snapshot_bytes.unwrap_or(256 * 1024 * 1024) as usize;
        if bytes.len() > maximum {
            return Err(napi::Error::from_reason("sqlite snapshot byte limit exceeded"));
        }
        validate_database(&bytes).map_err(|e| napi::Error::from_reason(e.to_string()))?;
        let mut snapshot = Vec::new();
        snapshot.try_reserve_exact(bytes.len()).map_err(|_| napi::Error::from_reason("sqlite snapshot allocation failed"))?;
        snapshot.extend_from_slice(&bytes);
        Ok(Self { bytes: snapshot })
    }

    /// Returns typed schema objects without parsing their SQL definitions.
    #[napi]
    pub fn schema(&self, limits: Option<SqliteReadLimits>) -> napi::Result<Vec<SqliteSchemaObject>> {
        read_schema(&self.bytes, SqliteReadLimits::resolve(limits))
            .map(|objects| {
                objects
                    .into_iter()
                    .map(|object| SqliteSchemaObject {
                        kind: match object.kind {
                            SchemaObjectKind::Table => "table",
                            SchemaObjectKind::Index => "index",
                            SchemaObjectKind::View => "view",
                            SchemaObjectKind::Trigger => "trigger",
                        }
                        .into(),
                        name: object.name,
                        table_name: object.table_name,
                        root_page: object.root_page,
                        sql: object.sql,
                    })
                    .collect()
            })
            .map_err(|e| napi::Error::from_reason(e.to_string()))
    }

    /// Reads raw records from an ordinary rowid table.
    #[napi]
    pub fn table(&self, name: String, limits: Option<SqliteReadLimits>) -> napi::Result<Vec<SqliteTableRow>> {
        read_named_table(&self.bytes, &name, SqliteReadLimits::resolve(limits))
            .map(|rows| {
                rows.into_iter()
                    .map(|row| SqliteTableRow { rowid: i64::from(row.rowid), payload: row.payload.into() })
                    .collect()
            })
            .map_err(|e| napi::Error::from_reason(e.to_string()))
    }

    /// Reads decoded typed records from an ordinary rowid table.
    #[napi]
    pub fn tableDecoded(&self, name: String, limits: Option<SqliteReadLimits>) -> napi::Result<Vec<SqliteDecodedRow>> {
        let resolved = SqliteReadLimits::resolve(limits);
        let rows = read_named_table(&self.bytes, &name, resolved).map_err(|e| napi::Error::from_reason(e.to_string()))?;
        build_decoded_rows(&self.bytes, rows, resolved)
    }

    /// SQLite library version stamped in the snapshot header.
    #[napi]
    pub fn sqliteVersion(&self) -> napi::Result<String> {
        decode_library_version(&self.bytes).map_err(|e| napi::Error::from_reason(e.to_string()))
    }

    /// Returns the decoded text encoding label (`utf8`, `utf16le`, or `utf16be`).
    #[napi]
    pub fn encoding(&self) -> napi::Result<String> {
        resolve_encoding_name(&self.bytes).map(|s| s.to_owned())
    }

    /// Health probe that re-validates the main-file format.
    #[napi]
    pub fn ping(&self) -> napi::Result<i32> {
        validate_database(&self.bytes).map_err(|e| napi::Error::from_reason(e.to_string()))?;
        Ok(1)
    }

    /// Returns a view over the raw main-file bytes.
    #[napi]
    pub fn pages(&self) -> napi::Result<Buffer> {
        Ok(self.bytes.clone().into())
    }
}

/// A file-backed or in-memory SQLite snapshot handle (main-file only, no WAL recovery).
///
/// Read-only inspection surface. SQL execution is intentionally absent —
/// host applications that need SELECT/INSERT use the YYDB engine instead.
#[napi]
pub struct SqliteDatabase {
    bytes: Vec<u8>,
    path: Option<std::path::PathBuf>,
}

#[napi]
impl SqliteDatabase {
    /// Opens a blank in-memory SQLite format-3 snapshot.
    #[napi(factory)]
    pub fn openInMemory() -> napi::Result<Self> {
        let bytes = blank_database().map_err(|e| napi::Error::from_reason(e.to_string()))?;
        Ok(Self { bytes, path: None })
    }

    /// Opens or creates a file-backed SQLite snapshot (main-file only).
    #[napi(factory)]
    pub fn open(path: String) -> napi::Result<Self> {
        let path_buf = std::path::PathBuf::from(&path);
        if path_buf.exists() {
            let bytes = read_existing(&path).map_err(|e| napi::Error::from_reason(e.to_string()))?;
            return Ok(Self { bytes, path: Some(path_buf) });
        }
        let bytes = blank_database().map_err(|e| napi::Error::from_reason(e.to_string()))?;
        Ok(Self { bytes, path: Some(path_buf) })
    }

    /// File path when file-backed, otherwise `null`.
    #[napi]
    pub fn path(&self) -> Option<String> {
        self.path.as_ref().map(|p| p.display().to_string())
    }

    /// Health probe that re-validates the main-file format.
    #[napi]
    pub fn ping(&self) -> napi::Result<i32> {
        validate_database(&self.bytes).map_err(|e| napi::Error::from_reason(e.to_string()))?;
        Ok(1)
    }

    /// SQLite library version stamped in the snapshot header.
    #[napi]
    pub fn sqliteVersion(&self) -> napi::Result<String> {
        decode_library_version(&self.bytes).map_err(|e| napi::Error::from_reason(e.to_string()))
    }

    /// Returns the decoded text encoding label.
    #[napi]
    pub fn encoding(&self) -> napi::Result<String> {
        resolve_encoding_name(&self.bytes).map(|s| s.to_owned())
    }

    /// Returns typed schema objects.
    #[napi]
    pub fn schema(&self, limits: Option<SqliteReadLimits>) -> napi::Result<Vec<SqliteSchemaObject>> {
        read_schema(&self.bytes, SqliteReadLimits::resolve(limits))
            .map(|objects| {
                objects
                    .into_iter()
                    .map(|object| SqliteSchemaObject {
                        kind: match object.kind {
                            SchemaObjectKind::Table => "table",
                            SchemaObjectKind::Index => "index",
                            SchemaObjectKind::View => "view",
                            SchemaObjectKind::Trigger => "trigger",
                        }
                        .into(),
                        name: object.name,
                        table_name: object.table_name,
                        root_page: object.root_page,
                        sql: object.sql,
                    })
                    .collect()
            })
            .map_err(|e| napi::Error::from_reason(e.to_string()))
    }

    /// Reads decoded typed records from an ordinary rowid table.
    #[napi]
    pub fn table(&self, name: String, limits: Option<SqliteReadLimits>) -> napi::Result<Vec<SqliteDecodedRow>> {
        let resolved = SqliteReadLimits::resolve(limits);
        let rows = read_named_table(&self.bytes, &name, resolved).map_err(|e| napi::Error::from_reason(e.to_string()))?;
        build_decoded_rows(&self.bytes, rows, resolved)
    }

    /// Reads raw records (undecoded payload) from an ordinary rowid table.
    #[napi]
    pub fn tableRaw(&self, name: String, limits: Option<SqliteReadLimits>) -> napi::Result<Vec<SqliteTableRow>> {
        read_named_table(&self.bytes, &name, SqliteReadLimits::resolve(limits))
            .map(|rows| {
                rows.into_iter()
                    .map(|row| SqliteTableRow { rowid: i64::from(row.rowid), payload: row.payload.into() })
                    .collect()
            })
            .map_err(|e| napi::Error::from_reason(e.to_string()))
    }

    /// Returns a view over the raw main-file bytes.
    #[napi]
    pub fn pages(&self) -> napi::Result<Buffer> {
        Ok(self.bytes.clone().into())
    }
}

/// Decodes one SQLite record payload into typed storage-class values.
#[napi]
pub fn decodeRecord(payload: Buffer, encoding: Option<String>) -> napi::Result<Vec<SqliteRecordValue>> {
    let encoding = encode_encoding(encoding.as_deref())?;
    let values =
        decode_record(&payload, encoding, RecordLimits { max_columns: 1024, max_payload_bytes: payload.len().max(1_048_576) })
            .map_err(|e| napi::Error::from_reason(e.to_string()))?;
    Ok(values.into_iter().map(record_value).collect())
}
