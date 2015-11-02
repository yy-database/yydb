//! Python surface for the SQLite format-3 product facade.
//!
//! Read-only inspection and snapshot writing only. SQL execution stays out of
//! scope here, matching the Rust `sqlite` facade level table.

use database::{
    formats::sqlite::{RecordLimits, RecordValue, decode_library_version, validate_database},
    storage::sqlite::{
        ColumnSpec, SchemaObjectKind, TableLimits, TextEncoding, UpdateValue, WriteValue, decode_record, read_named_index,
        read_named_table, read_named_without_rowid, read_schema,
    },
};
use pyo3::{
    exceptions::PyRuntimeError,
    prelude::*,
    types::{PyBytes, PyDict, PyList},
};

fn sqlite_error(error: impl ToString) -> PyErr {
    PyRuntimeError::new_err(error.to_string())
}

fn text_encoding(bytes: &[u8]) -> PyResult<TextEncoding> {
    let raw = bytes.get(56..60).ok_or_else(|| PyRuntimeError::new_err("sqlite snapshot has no text encoding"))?;
    match u32::from_be_bytes(raw.try_into().expect("four byte header slice")) {
        1 => Ok(TextEncoding::Utf8),
        2 => Ok(TextEncoding::Utf16Le),
        3 => Ok(TextEncoding::Utf16Be),
        _ => Err(PyRuntimeError::new_err("sqlite snapshot has unknown text encoding")),
    }
}

fn encoding_label(bytes: &[u8]) -> PyResult<&'static str> {
    match text_encoding(bytes)? {
        TextEncoding::Utf8 => Ok("utf8"),
        TextEncoding::Utf16Le => Ok("utf16le"),
        TextEncoding::Utf16Be => Ok("utf16be"),
    }
}

fn record_list(py: Python<'_>, values: Vec<RecordValue<'_>>) -> PyResult<Py<PyAny>> {
    let mut items = Vec::with_capacity(values.len());
    for value in values {
        let dict = PyDict::new(py);
        match value {
            RecordValue::Null => dict.set_item("kind", "null")?,
            RecordValue::Integer(number) => {
                dict.set_item("kind", "integer")?;
                dict.set_item("value", number)?;
            }
            RecordValue::Real(number) => {
                dict.set_item("kind", "real")?;
                dict.set_item("value", number)?;
            }
            RecordValue::Text(text) => {
                dict.set_item("kind", "text")?;
                dict.set_item("value", text)?;
            }
            RecordValue::Blob(bytes) => {
                dict.set_item("kind", "blob")?;
                dict.set_item("value", PyBytes::new(py, bytes))?;
            }
        }
        items.push(dict.into_any().unbind());
    }
    Ok(PyList::new(py, items)?.into_any().unbind())
}

fn decoded_rows(py: Python<'_>, bytes: &[u8], payloads: Vec<Vec<u8>>) -> PyResult<Py<PyAny>> {
    let encoding = text_encoding(bytes)?;
    let limits = RecordLimits { max_columns: 1024, max_payload_bytes: 1_048_576 };
    let mut items = Vec::with_capacity(payloads.len());
    for payload in payloads {
        let values = decode_record(&payload, encoding, limits).map_err(sqlite_error)?;
        items.push(record_list(py, values)?);
    }
    Ok(PyList::new(py, items)?.into_any().unbind())
}

fn table_rows(py: Python<'_>, bytes: &[u8], name: &str) -> PyResult<Py<PyAny>> {
    let rows = read_named_table(bytes, name, TableLimits::default()).map_err(sqlite_error)?;
    let encoding = text_encoding(bytes)?;
    let limits = RecordLimits { max_columns: 1024, max_payload_bytes: 1_048_576 };
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let values = decode_record(&row.payload, encoding, limits).map_err(sqlite_error)?;
        let dict = PyDict::new(py);
        dict.set_item("rowid", i64::from(row.rowid))?;
        dict.set_item("columns", record_list(py, values)?)?;
        items.push(dict.into_any().unbind());
    }
    Ok(PyList::new(py, items)?.into_any().unbind())
}

fn index_rows(py: Python<'_>, bytes: &[u8], name: &str) -> PyResult<Py<PyAny>> {
    let payloads = read_named_index(bytes, name, TableLimits::default()).map_err(sqlite_error)?;
    let encoding = text_encoding(bytes)?;
    let limits = RecordLimits { max_columns: 1024, max_payload_bytes: 1_048_576 };
    let mut items = Vec::with_capacity(payloads.len());
    for payload in payloads {
        let mut values = decode_record(&payload, encoding, limits).map_err(sqlite_error)?;
        let trailing = values.pop().ok_or_else(|| PyRuntimeError::new_err("sqlite index record has no rowid column"))?;
        let rowid = match trailing {
            RecordValue::Integer(rowid) => rowid,
            _ => return Err(PyRuntimeError::new_err("sqlite index record rowid is not an integer")),
        };
        let dict = PyDict::new(py);
        dict.set_item("rowid", rowid)?;
        dict.set_item("columns", record_list(py, values)?)?;
        items.push(dict.into_any().unbind());
    }
    Ok(PyList::new(py, items)?.into_any().unbind())
}

fn schema_list(py: Python<'_>, bytes: &[u8]) -> PyResult<Py<PyAny>> {
    let objects = read_schema(bytes, TableLimits::default()).map_err(sqlite_error)?;
    let mut items = Vec::with_capacity(objects.len());
    for object in objects {
        let dict = PyDict::new(py);
        dict.set_item(
            "kind",
            match object.kind {
                SchemaObjectKind::Table => "table",
                SchemaObjectKind::Index => "index",
                SchemaObjectKind::View => "view",
                SchemaObjectKind::Trigger => "trigger",
            },
        )?;
        dict.set_item("name", object.name)?;
        dict.set_item("table_name", object.table_name)?;
        dict.set_item("root_page", object.root_page)?;
        dict.set_item("sql", object.sql)?;
        items.push(dict.into_any().unbind());
    }
    Ok(PyList::new(py, items)?.into_any().unbind())
}

fn json_to_write_value(json: &serde_json::Value) -> PyResult<WriteValue> {
    Ok(match json {
        serde_json::Value::Null => WriteValue::Null,
        serde_json::Value::Bool(flag) => WriteValue::Integer(i64::from(*flag)),
        serde_json::Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                WriteValue::Integer(integer)
            }
            else if let Some(real) = number.as_f64() {
                WriteValue::Real(real)
            }
            else {
                return Err(PyRuntimeError::new_err("sqlite write value number out of range"));
            }
        }
        serde_json::Value::String(text) => WriteValue::Text(text.clone()),
        serde_json::Value::Object(object) => {
            if let Some(hex) = object.get("$blob").and_then(serde_json::Value::as_str) {
                if hex.len() % 2 != 0 {
                    return Err(PyRuntimeError::new_err("sqlite blob hex must have even length"));
                }
                let mut bytes = Vec::with_capacity(hex.len() / 2);
                for index in 0..hex.len() / 2 {
                    let byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
                        .map_err(|_| PyRuntimeError::new_err("sqlite blob hex is not valid"))?;
                    bytes.push(byte);
                }
                WriteValue::Blob(bytes)
            }
            else if let Some(integer) = object.get("$i64").and_then(serde_json::Value::as_str) {
                let value =
                    integer.parse::<i64>().map_err(|_| PyRuntimeError::new_err("sqlite $i64 must be a decimal string"))?;
                WriteValue::Integer(value)
            }
            else {
                return Err(PyRuntimeError::new_err("sqlite write value object must carry `$blob` or `$i64`"));
            }
        }
        serde_json::Value::Array(_) => return Err(PyRuntimeError::new_err("sqlite write value must not be an array")),
    })
}

fn columns_from_json(columns_json: &str) -> PyResult<Vec<ColumnSpec>> {
    let columns: Vec<serde_json::Value> = serde_json::from_str(columns_json)
        .map_err(|error| PyRuntimeError::new_err(format!("sqlite columns JSON invalid: {error}")))?;
    let mut specs = Vec::with_capacity(columns.len());
    for column in columns {
        let name = column
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| PyRuntimeError::new_err("sqlite column needs `name`"))?;
        specs.push(ColumnSpec {
            name: name.to_owned(),
            declared_type: column.get("declaredType").and_then(serde_json::Value::as_str).unwrap_or("BLOB").to_owned(),
            not_null: column.get("notNull").and_then(serde_json::Value::as_bool).unwrap_or(false),
            default_value: match column.get("defaultValue") {
                Some(serde_json::Value::Null) | None => None,
                Some(json) => Some(json_to_write_value(json)?),
            },
            primary_key: column.get("primaryKey").and_then(serde_json::Value::as_bool).unwrap_or(false),
            unique: column.get("unique").and_then(serde_json::Value::as_bool).unwrap_or(false),
            autoincrement: false,
        });
    }
    Ok(specs)
}

/// Writes SQLite format-3 database images that upstream engines open directly.
#[pyclass(module = "yydb_pyo3")]
pub struct SqliteWriter {
    writer: database::storage::sqlite::SqliteWriter,
}

#[pymethods]
impl SqliteWriter {
    /// Creates a fresh database image with the given page size (default 4096).
    #[staticmethod]
    #[pyo3(signature = (page_size=None))]
    fn create(page_size: Option<usize>) -> PyResult<Self> {
        let writer = database::storage::sqlite::SqliteWriter::create(page_size.unwrap_or(4096)).map_err(sqlite_error)?;
        Ok(Self { writer })
    }

    /// Loads an existing SQLite main file for further mutation.
    #[staticmethod]
    fn open(path: &str) -> PyResult<Self> {
        let writer = database::storage::sqlite::SqliteWriter::open(path).map_err(sqlite_error)?;
        Ok(Self { writer })
    }

    /// Creates a table from the same column JSON shape as `sqlite.ts`.
    fn create_table(&mut self, name: &str, columns_json: &str) -> PyResult<()> {
        let specs = columns_from_json(columns_json)?;
        self.writer.create_table(name, &specs).map_err(sqlite_error)
    }

    /// Creates an index over one or more columns.
    #[pyo3(signature = (name, table, columns, unique=false))]
    fn create_index(&mut self, name: &str, table: &str, columns: Vec<String>, unique: bool) -> PyResult<()> {
        self.writer.create_index(name, table, &columns, unique).map_err(sqlite_error)
    }

    /// Inserts one row and returns its rowid.
    fn insert(&mut self, table: &str, values_json: &str) -> PyResult<i64> {
        let values: Vec<serde_json::Value> = serde_json::from_str(values_json)
            .map_err(|error| PyRuntimeError::new_err(format!("sqlite values JSON invalid: {error}")))?;
        let mut slots = Vec::with_capacity(values.len());
        for value in values {
            if value.as_object().is_some_and(|object| object.get("$omit").and_then(serde_json::Value::as_bool).unwrap_or(false))
            {
                slots.push(None);
                continue;
            }
            slots.push(Some(json_to_write_value(&value)?));
        }
        self.writer.insert(table, &slots).map_err(sqlite_error)
    }

    /// Deletes one row by rowid.
    fn delete(&mut self, table: &str, rowid: i64) -> PyResult<bool> {
        self.writer.delete(table, rowid).map_err(sqlite_error)
    }

    /// Updates one row by rowid with `keep` and `set` slots.
    fn update(&mut self, table: &str, rowid: i64, slots_json: &str) -> PyResult<bool> {
        let values: Vec<serde_json::Value> = serde_json::from_str(slots_json)
            .map_err(|error| PyRuntimeError::new_err(format!("sqlite slots JSON invalid: {error}")))?;
        let mut slots = Vec::with_capacity(values.len());
        for value in values {
            let object = value.as_object().ok_or_else(|| PyRuntimeError::new_err("sqlite update slot must be an object"))?;
            if object.get("keep").and_then(serde_json::Value::as_bool).unwrap_or(false) {
                slots.push(UpdateValue::Keep);
                continue;
            }
            match object.get("set") {
                Some(inner) => slots.push(UpdateValue::Set(json_to_write_value(inner)?)),
                None => return Err(PyRuntimeError::new_err("sqlite update slot needs `keep` or `set`")),
            }
        }
        self.writer.update(table, rowid, &slots).map_err(sqlite_error)
    }

    /// Starts an in-memory transaction.
    fn begin(&mut self) -> PyResult<()> {
        self.writer.begin().map_err(sqlite_error)
    }

    /// Commits the open transaction.
    fn commit(&mut self) -> PyResult<()> {
        self.writer.commit().map_err(sqlite_error)
    }

    /// Rolls the image back to the state before `begin`.
    fn rollback(&mut self) -> PyResult<()> {
        self.writer.rollback().map_err(sqlite_error)
    }

    /// Writes the image to `path`.
    fn save(&mut self, path: &str) -> PyResult<()> {
        self.writer.save(path).map_err(sqlite_error)
    }

    /// Returns the raw file image.
    fn pages<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.writer.pages())
    }

    /// Page size of this database image.
    fn page_size(&self) -> usize {
        self.writer.page_size()
    }

    /// Reads one table back through the crate's own reader.
    fn table_decoded(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        let rows = self.writer.table_rows(name, TableLimits::default()).map_err(sqlite_error)?;
        let encoding = text_encoding(self.writer.pages())?;
        let limits = RecordLimits { max_columns: 1024, max_payload_bytes: 1_048_576 };
        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            let values = decode_record(&row.payload, encoding, limits).map_err(sqlite_error)?;
            let dict = PyDict::new(py);
            dict.set_item("rowid", i64::from(row.rowid))?;
            dict.set_item("columns", record_list(py, values)?)?;
            items.push(dict.into_any().unbind());
        }
        Ok(PyList::new(py, items)?.into_any().unbind())
    }
}

/// Read-only handle over a SQLite main file with WAL and journal recovery.
#[pyclass(module = "yydb_pyo3")]
pub struct SqliteDatabase {
    bytes: Vec<u8>,
    path: Option<String>,
}

#[pymethods]
impl SqliteDatabase {
    /// Opens a blank in-memory SQLite format-3 snapshot.
    #[staticmethod]
    fn open_in_memory() -> PyResult<Self> {
        let database = database::storage::sqlite::SqliteDatabase::open_in_memory().map_err(sqlite_error)?;
        Ok(Self { bytes: database.pages().to_vec(), path: None })
    }

    /// Opens or creates a file-backed database, replaying `-wal` or a hot `-journal`.
    #[staticmethod]
    fn open(path: &str) -> PyResult<Self> {
        let database = database::storage::sqlite::SqliteDatabase::open(path).map_err(sqlite_error)?;
        Ok(Self { bytes: database.pages().to_vec(), path: Some(path.to_owned()) })
    }

    /// File path when file-backed, otherwise `None`.
    fn path(&self) -> Option<String> {
        self.path.clone()
    }

    /// Health probe that re-validates the main-file format.
    fn ping(&self) -> PyResult<bool> {
        validate_database(&self.bytes).map_err(sqlite_error)?;
        Ok(true)
    }

    /// SQLite library version stamped in the header.
    fn sqlite_version(&self) -> PyResult<String> {
        decode_library_version(&self.bytes).map_err(sqlite_error)
    }

    /// Decoded text encoding label (`utf8`, `utf16le`, or `utf16be`).
    fn encoding(&self) -> PyResult<String> {
        encoding_label(&self.bytes).map(str::to_owned)
    }

    /// Typed schema objects.
    fn schema(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        schema_list(py, &self.bytes)
    }

    /// Decoded typed records from an ordinary rowid table.
    fn table(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        table_rows(py, &self.bytes, name)
    }

    /// Decoded records from a WITHOUT ROWID table in primary-key order.
    fn table_without_rowid(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        let payloads = read_named_without_rowid(&self.bytes, name, TableLimits::default()).map_err(sqlite_error)?;
        decoded_rows(py, &self.bytes, payloads)
    }

    /// Decoded entries from a secondary index in key order.
    fn index(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        index_rows(py, &self.bytes, name)
    }

    /// Returns the raw main-file bytes.
    fn pages<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.bytes)
    }
}
