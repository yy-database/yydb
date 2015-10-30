//! Thin Python binding — delegates to the [`database`] facade.
//!
//! TypeScript hosts use `yydb-napi`. Do not import `yydb-types` from Python glue.

#![deny(missing_docs)]

use std::{collections::BTreeMap, fs, path::Path, sync::Arc};

mod sqlite_bindings;

use database::types::Value;
use pyo3::{
    IntoPyObjectExt,
    exceptions::{PyRuntimeError, PyTypeError},
    prelude::*,
    types::{PyBool, PyBytes, PyDict, PyString},
};

/// Upstream diagnostic namespace produced by `database::types::Error::code`.
const UPSTREAM_CODE_PREFIX: &str = "database.";

/// Host-facing diagnostic namespace for the same errors.
const HOST_CODE_PREFIX: &str = "yydb.";

/// Rewrites the upstream code namespace to the host contract spelling.
fn host_error_message(message: &str) -> String {
    message.replace(UPSTREAM_CODE_PREFIX, HOST_CODE_PREFIX)
}

fn map_err(error: impl ToString) -> PyErr {
    PyRuntimeError::new_err(host_error_message(&error.to_string()))
}

/// Tagged JSON key exposed to the Python host contract.
const HOST_TAGGED_JSON_KEY: &str = "$yydb";

/// Rewrites a host tagged object to the canonical `database` key.
///
/// `database::types::scalar::tagged_value_from_json` reads only
/// `TAGGED_JSON_KEY`; the `$yydb` spelling stays accepted as a compatibility
/// input, while `$database` wins when both keys are present.
fn retag_host_to_database(json: serde_json::Value) -> serde_json::Value {
    match json {
        serde_json::Value::Object(mut map) => {
            if !map.contains_key(database::types::scalar::TAGGED_JSON_KEY) {
                if let Some(tag) = map.remove(HOST_TAGGED_JSON_KEY) {
                    map.insert(database::types::scalar::TAGGED_JSON_KEY.to_owned(), tag);
                }
            }
            serde_json::Value::Object(map)
        }
        other => other,
    }
}

/// Rewrites the canonical tagged key to the `$yydb` spelling the Python host
/// contract requires. Only the key name changes; the value structure is kept.
fn retag_database_to_host(mut json: serde_json::Value) -> serde_json::Value {
    if let serde_json::Value::Object(map) = &mut json {
        if let Some(tag) = map.remove(database::types::scalar::TAGGED_JSON_KEY) {
            map.insert(HOST_TAGGED_JSON_KEY.to_owned(), tag);
        }
    }
    json
}

/// Whether `dict` carries either tagged spelling.
fn carries_tagged_value(dict: &Bound<'_, PyDict>) -> PyResult<bool> {
    Ok(dict.contains(database::types::scalar::TAGGED_JSON_KEY)? || dict.contains(HOST_TAGGED_JSON_KEY)?)
}

/// Host value accepted by [`execute_vos_write_with_params`].
///
/// `bytes` stays `bytes` in both directions: it is never decoded as text and
/// never re-encoded, so a payload that no text spelling can carry survives the
/// host boundary unchanged. A tagged dict carrying `"$yydb"` (or the canonical
/// `"$database"`) is decoded into the matching scalar, including
/// [`database::types::Value::File`] descriptors so VOS `file` columns can be
/// round-tripped through the Python host.
fn py_to_value(value: &Bound<'_, PyAny>) -> PyResult<Value> {
    if value.is_none() {
        return Ok(Value::Null);
    }
    if let Ok(payload) = value.cast::<PyBytes>() {
        return Ok(Value::Bytes(payload.as_bytes().to_vec()));
    }
    if value.cast::<PyBool>().is_ok() {
        return Ok(Value::Bool(value.extract::<bool>()?));
    }
    if let Ok(dict) = value.cast::<PyDict>() {
        if carries_tagged_value(dict)? {
            let json = retag_host_to_database(dict_to_json(dict)?);
            if let Some(result) = database::types::scalar::tagged_value_from_json(&json) {
                return result.map_err(|error| PyRuntimeError::new_err(error.to_string()));
            }
        }
    }
    if let Ok(text) = value.cast::<PyString>() {
        return Ok(Value::Text(text.to_str()?.to_owned()));
    }
    if let Ok(number) = value.extract::<i64>() {
        return Ok(Value::I64(number));
    }
    if let Ok(number) = value.extract::<f64>() {
        return Ok(Value::F64(number));
    }
    Err(PyTypeError::new_err(format!("unsupported parameter value {value:?}")))
}

fn dict_to_json(dict: &Bound<'_, PyDict>) -> PyResult<serde_json::Value> {
    let mut map = serde_json::Map::new();
    for (key, value) in dict.iter() {
        let k = key.extract::<String>()?;
        let v = py_to_json(value)?;
        map.insert(k, v);
    }
    Ok(serde_json::Value::Object(map))
}

fn py_to_json(value: Bound<'_, PyAny>) -> PyResult<serde_json::Value> {
    if value.is_none() {
        return Ok(serde_json::Value::Null);
    }
    if let Ok(b) = value.extract::<bool>() {
        return Ok(serde_json::Value::Bool(b));
    }
    if let Ok(i) = value.extract::<i64>() {
        return Ok(serde_json::Value::Number(i.into()));
    }
    if let Ok(s) = value.extract::<String>() {
        return Ok(serde_json::Value::String(s));
    }
    if let Ok(dict) = value.cast::<PyDict>() {
        return dict_to_json(&dict);
    }
    Err(PyTypeError::new_err(format!("cannot encode {value:?} as JSON member")))
}

/// Reads one result value back into the matching Python object.
fn value_to_py(py: Python<'_>, value: Value) -> PyResult<Py<PyAny>> {
    match value {
        Value::Null => Ok(py.None()),
        Value::Bool(flag) => flag.into_py_any(py),
        Value::I64(number) => number.into_py_any(py),
        Value::F64(number) => number.into_py_any(py),
        Value::Text(text) => text.into_py_any(py),
        Value::Bytes(octets) => PyBytes::new(py, &octets).into_py_any(py),
        other => {
            // Tagged scalar forms (uuid, date, time, timestamp, decimal,
            // vector, wide integers, file descriptors) cross as tagged dicts
            // exactly like the TypeScript host.
            match database::types::scalar::tagged_value_json(&other) {
                Some(json) => json_to_py(py, &retag_database_to_host(json)),
                None => Err(PyTypeError::new_err(format!("unsupported result value `{}`", other.kind()))),
            }
        }
    }
}

fn json_to_py(py: Python<'_>, json: &serde_json::Value) -> PyResult<Py<PyAny>> {
    match json {
        serde_json::Value::Null => Ok(py.None()),
        serde_json::Value::Bool(b) => b.into_py_any(py),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.into_py_any(py)
            }
            else if let Some(f) = n.as_f64() {
                f.into_py_any(py)
            }
            else {
                Err(PyTypeError::new_err(format!("cannot encode number {n}")))
            }
        }
        serde_json::Value::String(s) => s.into_py_any(py),
        serde_json::Value::Array(items) => {
            let list = pyo3::types::PyList::empty(py);
            for item in items {
                list.append(json_to_py(py, item)?)?;
            }
            Ok(list.into_any().unbind())
        }
        serde_json::Value::Object(members) => {
            let dict = PyDict::new(py);
            for (k, v) in members {
                dict.set_item(k, json_to_py(py, v)?)?;
            }
            Ok(dict.into_any().unbind())
        }
    }
}

/// Embedded `yydb` engine version string.
#[pyfunction]
fn version() -> String {
    database::execution::engine::version().to_string()
}

/// Run the YYDB serve loop on `bind` for the database at `db_path`.
#[pyfunction]
#[pyo3(signature = (db_path, bind, insecure_bind=None))]
fn serve(db_path: String, bind: String, insecure_bind: Option<bool>) -> PyResult<()> {
    database::server::yydb::run_serve(Path::new(&db_path), &bind, insecure_bind.unwrap_or(false)).map_err(map_err)?;
    Ok(())
}

/// Open `db_path` and, when a document is given, ensure the VOS schema.
///
/// A new database stays schema-less until a document is provided, so callers
/// can open a handle first and apply the schema afterwards.
#[pyfunction]
#[pyo3(signature = (db_path, schema_document=None))]
fn init_db(db_path: String, schema_document: Option<String>) -> PyResult<()> {
    let conn = database::execution::engine::Connection::open(&db_path).map_err(map_err)?;
    conn.schema().map_err(map_err)?;
    if let Some(document) = schema_document {
        conn.ensure_schema(&document).map_err(map_err)?;
    }
    Ok(())
}

/// Return multi-line database diagnostics for `db_path`.
#[pyfunction]
fn info_text(db_path: String) -> PyResult<String> {
    let conn = database::execution::engine::Connection::open(&db_path).map_err(map_err)?;
    let path_display = conn.path().map(|path| path.display().to_string()).unwrap_or_else(|| "<memory>".to_owned());
    let mut lines = vec![format!("path={path_display}")];
    match conn.schema().map_err(map_err)? {
        Some(schema) => {
            lines.push(format!("schema.version={}", schema.version));
            lines.push(format!("schema.document={}", summarize_document(&schema.document)));
        }
        None => lines.push("schema=(none)".to_owned()),
    }
    lines.push(format!("records={}", conn.record_count().map_err(map_err)?));
    lines.push(format!("journal_mode={}", conn.journal_mode().as_str()));
    if let (Some(wal), Some(shm)) = (conn.wal_path(), conn.shm_path()) {
        let (wal_on, shm_on, frames) =
            database::execution::engine::journal::sidecar_status(conn.path().expect("file-backed")).map_err(map_err)?;
        lines.push(format!("wal={} exists={wal_on}", wal.display()));
        lines.push(format!("shm={} exists={shm_on}", shm.display()));
        lines.push(format!("wal_frames={frames}"));
    }
    Ok(lines.join("\n"))
}

/// Read a VOS schema document from disk.
#[pyfunction]
fn read_schema_file(schema_path: String) -> PyResult<String> {
    fs::read_to_string(&schema_path).map_err(map_err)
}

/// Execute a VOS read pipeline and return one dict per row.
///
/// A `bytes` cell is returned as Python `bytes` holding the stored octets, not
/// as text: no host-side decoding happens, so a payload that is not valid UTF-8
/// survives the boundary unchanged.
#[pyfunction]
fn query_vos(py: Python<'_>, db_path: String, source: String) -> PyResult<Vec<Py<PyAny>>> {
    let conn = database::execution::engine::Connection::open(&db_path).map_err(map_err)?;
    let rows = conn.query_vos(&source).map_err(map_err)?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let dict = PyDict::new(py);
        for (column, value) in row {
            dict.set_item(column, value_to_py(py, value)?)?;
        }
        out.push(dict.into_any().unbind());
    }
    Ok(out)
}

/// Execute one VOS write statement with named parameters.
///
/// `bytes` parameters stay `bytes`: they are handed to the engine as octets and
/// never pass through a text spelling. A dict tagged with `"$yydb"` or the
/// canonical `"$database"` is accepted as its matching scalar, including
/// `Value::File` descriptors for VOS `file` columns.
#[pyfunction]
fn execute_vos_write(db_path: String, source: String, params: &Bound<'_, PyDict>) -> PyResult<()> {
    let conn = database::execution::engine::Connection::open(&db_path).map_err(map_err)?;
    let mut named = BTreeMap::new();
    for (key, value) in params.iter() {
        named.insert(key.extract::<String>()?, py_to_value(&value)?);
    }
    conn.execute_vos_write_with_params(&source, &named).map_err(map_err)
}

/// Execute a batch of VOS write statements inside one transaction.
///
/// Every statement runs against one connection under a single transaction: a
/// failure anywhere rolls the whole batch back, otherwise it commits. This is
/// the transactional surface of the stateless Python handle, which opens a
/// fresh connection per native call by design.
#[pyfunction]
fn execute_vos_transaction(db_path: String, statements: Vec<(String, Bound<'_, PyDict>)>) -> PyResult<()> {
    if statements.is_empty() {
        return Err(PyTypeError::new_err("transaction batch must not be empty"));
    }
    let conn = database::execution::engine::Connection::open(&db_path).map_err(map_err)?;
    conn.begin().map_err(map_err)?;
    for (source, params) in &statements {
        let mut named = BTreeMap::new();
        for (key, value) in params.iter() {
            named.insert(key.extract::<String>()?, py_to_value(&value)?);
        }
        if let Err(error) = conn.execute_vos_write_with_params(source, &named) {
            let _ = conn.rollback();
            return Err(map_err(error));
        }
    }
    conn.commit().map_err(map_err)
}

fn decode_descriptor_dict(dict: &Bound<'_, PyDict>) -> PyResult<database::types::FileDescriptor> {
    let json = retag_host_to_database(dict_to_json(dict)?);
    let decoded = database::types::scalar::tagged_value_from_json(&json)
        .ok_or_else(|| PyRuntimeError::new_err("dict does not carry a tagged scalar value"))?
        .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
    match decoded {
        Value::File(descriptor) => Ok(descriptor),
        other => Err(PyRuntimeError::new_err(format!("expected a `file` tagged value, got `{}`", other.kind()))),
    }
}

/// Publish a logical `file` value and return the descriptor as a tagged
/// `{"$yydb":"file",…}` dict.
///
/// Both layouts support it: `.yydx` keeps the object tree in
/// `<stem>-objects/`, and a single-file `.yydb` embeds the same objects as
/// records in its main image. Embedding large payloads inflates the single
/// file and slows down snapshots and backups, so a `.yydx` layout is
/// recommended for object-heavy workloads.
#[pyfunction]
#[pyo3(signature = (db_path, payload, chunk_size, name=None, media_type=None))]
fn put_file(
    py: Python<'_>,
    db_path: String,
    payload: &[u8],
    chunk_size: u32,
    name: Option<String>,
    media_type: Option<String>,
) -> PyResult<Py<PyAny>> {
    let conn = database::execution::engine::Connection::open(&db_path).map_err(map_err)?;
    let chunk_size =
        usize::try_from(chunk_size).map_err(|_| PyRuntimeError::new_err("chunk_size exceeds platform pointer width"))?;
    let descriptor = conn.put_file(std::io::Cursor::new(payload), chunk_size, name, media_type).map_err(map_err)?;
    let json =
        database::types::scalar::tagged_value_json(&Value::File(descriptor)).expect("`Value::File` always has a tagged form");
    json_to_py(py, &retag_database_to_host(json))
}

/// Read one VOS window of a committed `file` value.
///
/// `descriptor` is a tagged `{"$yydb":"file",…}` dict — the shape produced by
/// [`put_file`] or queried from a VOS `file` column. The canonical
/// `"$database"` spelling is accepted as well.
#[pyfunction]
#[pyo3(signature = (db_path, descriptor, offset, length))]
fn read_file_window(
    py: Python<'_>,
    db_path: String,
    descriptor: &Bound<'_, PyDict>,
    offset: i64,
    length: i64,
) -> PyResult<Py<PyBytes>> {
    let conn = database::execution::engine::Connection::open(&db_path).map_err(map_err)?;
    let descriptor = decode_descriptor_dict(descriptor)?;
    let offset = u64::try_from(offset).map_err(|_| PyRuntimeError::new_err("offset must be non-negative"))?;
    let length = u64::try_from(length).map_err(|_| PyRuntimeError::new_err("length must be non-negative"))?;
    let octets = conn.read_file_window(&descriptor, vos::types::FileReadWindow::new(offset, length)).map_err(map_err)?;
    Ok(PyBytes::new(py, &octets).into())
}

/// Read the complete octets of a committed `file` value and verify its
/// content digest.
#[pyfunction]
fn read_file(py: Python<'_>, db_path: String, descriptor: &Bound<'_, PyDict>) -> PyResult<Py<PyBytes>> {
    let conn = database::execution::engine::Connection::open(&db_path).map_err(map_err)?;
    let descriptor = decode_descriptor_dict(descriptor)?;
    let octets = conn.read_file(&descriptor).map_err(map_err)?;
    Ok(PyBytes::new(py, &octets).into())
}

/// One open embedded handle that keeps transaction state across calls.
///
/// The stateless module functions in this extension open a fresh connection per
/// call, so they cannot span a transaction. Use this class when several writes,
/// a savepoint or a rollback must share one open handle.
#[pyclass(name = "Connection")]
pub struct PyConnection {
    conn: Arc<database::execution::engine::Connection>,
}

impl PyConnection {
    fn from_conn(conn: database::execution::engine::Connection) -> Self {
        Self { conn: Arc::new(conn) }
    }

    fn restore_handle(backup: &str) -> PyResult<Self> {
        let conn = if backup.ends_with(".yydx") {
            database::execution::engine::Connection::restore_yydx(backup)
        }
        else {
            database::execution::engine::Connection::restore_yydb(backup)
        };
        Ok(Self::from_conn(conn.map_err(map_err)?))
    }

    fn compact_report(&self) -> PyResult<String> {
        let report = self.conn.compact_yydx().map_err(map_err)?;
        serde_json::to_string(&report).map_err(|error| PyRuntimeError::new_err(error.to_string()))
    }
}

#[pymethods]
impl PyConnection {
    /// Open `path`, optionally with the `YYWL` journal enabled.
    #[new]
    #[pyo3(signature = (path, wal = true))]
    fn new(path: &str, wal: bool) -> PyResult<Self> {
        let flags =
            if wal { database::execution::engine::OpenFlags::wal() } else { database::execution::engine::OpenFlags::new() };
        Ok(Self::from_conn(database::execution::engine::Connection::open_with_flags(path, flags).map_err(map_err)?))
    }

    /// Open `path` with the default (delete) journal.
    #[staticmethod]
    fn open(path: &str) -> PyResult<Self> {
        Ok(Self::from_conn(database::execution::engine::Connection::open(path).map_err(map_err)?))
    }

    /// Open a `.yydx` database with the sibling object tree.
    #[staticmethod]
    fn open_yydx(path: &str) -> PyResult<Self> {
        Ok(Self::from_conn(database::execution::engine::Connection::open_yydx(path).map_err(map_err)?))
    }

    /// Open an in-memory database.
    #[staticmethod]
    fn open_in_memory() -> PyResult<Self> {
        Ok(Self::from_conn(database::execution::engine::Connection::open_in_memory().map_err(map_err)?))
    }

    /// Open a verified backup copy: `.yydb` checks page integrity, `.yydx`
    /// checks the object tree closure and file digests.
    #[staticmethod]
    fn restore(backup: &str) -> PyResult<Self> {
        Self::restore_handle(backup)
    }

    /// Close the handle. Repeated calls are no-ops.
    fn close(&self) {
        self.conn.close();
    }

    /// Whether this handle has been closed.
    fn is_closed(&self) -> bool {
        self.conn.is_closed()
    }

    /// Main database path, or `None` for an in-memory handle.
    fn path(&self) -> Option<String> {
        self.conn.path().map(|path| path.display().to_string())
    }

    /// Reserved `-wal` companion path.
    ///
    /// The path is derived from the main file and is **not** guaranteed to exist:
    /// the sidecar appears only when WAL mode is enabled and a commit appended
    /// frames. Do not use this to test for durability state, use
    /// `journal_mode()` or `wal_frame_count()` instead.
    fn wal_path(&self) -> Option<String> {
        self.conn.wal_path().map(|path| path.display().to_string())
    }

    /// Reserved `-shm` companion path, subject to the same existence caveat as
    /// `wal_path()`.
    fn shm_path(&self) -> Option<String> {
        self.conn.shm_path().map(|path| path.display().to_string())
    }

    /// Active journal mode (`delete` or `wal`).
    fn journal_mode(&self) -> String {
        self.conn.journal_mode().as_str().to_owned()
    }

    /// Append frames present in the WAL sidecar.
    fn wal_frame_count(&self) -> PyResult<u32> {
        self.conn.wal_frame_count().map_err(map_err)
    }

    /// Fold the WAL into the main file.
    fn checkpoint(&self) -> PyResult<()> {
        self.conn.checkpoint().map_err(map_err)
    }

    /// Published schema revision, or `None` before the first schema.
    fn schema_version(&self) -> PyResult<Option<u32>> {
        self.conn.schema_version().map_err(map_err)
    }

    /// Number of committed records.
    fn record_count(&self) -> PyResult<usize> {
        self.conn.record_count().map_err(map_err)
    }

    /// Read one record, or `None` when the key is absent.
    fn get(&self, py: Python<'_>, key: &str) -> PyResult<Option<Py<PyBytes>>> {
        Ok(self.conn.get(key).map_err(map_err)?.map(|octets| PyBytes::new(py, &octets).into()))
    }

    /// Write one record.
    fn put(&self, key: &str, value: &[u8]) -> PyResult<()> {
        self.conn.put(key, value).map_err(map_err)
    }

    /// Delete one record. Absent keys succeed silently.
    fn delete(&self, key: &str) -> PyResult<()> {
        self.conn.write_batch(&[(key.to_owned(), None)]).map_err(map_err)
    }

    /// Scan keys in lexical order with `prefix` (`""` scans every key).
    ///
    /// `after` is an exclusive continuation cursor, pass `None` for the first
    /// page. Returns `(key, value)` pairs in scan order.
    #[pyo3(signature = (prefix, after = None, limit = 1000))]
    fn scan(&self, py: Python<'_>, prefix: &str, after: Option<&str>, limit: usize) -> PyResult<Vec<(String, Py<PyBytes>)>> {
        let rows = self.conn.scan_prefix(prefix, after, limit).map_err(map_err)?;
        Ok(rows.into_iter().map(|(key, value)| (key, PyBytes::new(py, &value).into())).collect())
    }

    /// Atomically write or delete many records in one commit.
    ///
    /// Each entry is `(key, value)` where `value=None` deletes the key.
    fn write_batch(&self, changes: Vec<(String, Option<Vec<u8>>)>) -> PyResult<()> {
        self.conn.write_batch(&changes).map_err(map_err)
    }

    /// Begin an explicit transaction, balanced by `commit()` or `rollback()`.
    fn begin(&self) -> PyResult<()> {
        self.conn.begin().map_err(map_err)
    }

    /// Commit the open transaction.
    fn commit(&self) -> PyResult<()> {
        self.conn.commit().map_err(map_err)
    }

    /// Roll the open transaction back.
    fn rollback(&self) -> PyResult<()> {
        self.conn.rollback().map_err(map_err)
    }

    /// Create a named savepoint inside the open transaction.
    fn savepoint(&self, name: &str) -> PyResult<()> {
        self.conn.savepoint(name).map_err(map_err)
    }

    /// Roll back to a savepoint, discarding writes made after it.
    fn rollback_to(&self, name: &str) -> PyResult<()> {
        self.conn.rollback_to_savepoint(name).map_err(map_err)
    }

    /// Release a savepoint so it can no longer be rolled back to.
    fn release(&self, name: &str) -> PyResult<()> {
        self.conn.release_savepoint(name).map_err(map_err)
    }

    /// Run read-only consistency probes and return the report as JSON.
    ///
    /// The report only *finds* damage. This method does not repair a corrupt
    /// page, a torn WAL frame or a missing object: recovery is open-time
    /// truncation of an uncommitted WAL tail, and a data page that fails CRC32C
    /// stays failed. Restore from a verified backup instead.
    fn doctor(&self) -> PyResult<String> {
        let report = self.conn.doctor().map_err(map_err)?;
        serde_json::to_string(&report).map_err(|error| PyRuntimeError::new_err(error.to_string()))
    }

    /// Checkpoint, then copy a self-contained main file to `dest`.
    fn backup(&self, dest: &str) -> PyResult<()> {
        self.conn.backup_to(dest).map_err(map_err)
    }

    /// Fold the WAL and reclaim safe orphan objects (`.yydx` layouts only).
    ///
    /// Returns the compact report as JSON. A single-file `.yydb` handle returns
    /// an error because it has no object tree to compact.
    fn compact(&self) -> PyResult<String> {
        self.compact_report()
    }
}

fn open_default(path: &str) -> PyResult<PyConnection> {
    PyConnection::open(path)
}

/// Write one record through a stateless handle.
#[pyfunction]
fn kv_put(db_path: String, key: String, value: &[u8]) -> PyResult<()> {
    open_default(&db_path)?.put(&key, value)
}

/// Read one record, or `None` when the key is absent.
#[pyfunction]
fn kv_get(py: Python<'_>, db_path: String, key: String) -> PyResult<Option<Py<PyBytes>>> {
    open_default(&db_path)?.get(py, &key)
}

/// Delete one record. Absent keys succeed silently.
#[pyfunction]
fn kv_delete(db_path: String, key: String) -> PyResult<()> {
    open_default(&db_path)?.delete(&key)
}

/// Scan keys in lexical order with `prefix`, returning `(key, value)` pairs.
#[pyfunction]
#[pyo3(signature = (db_path, prefix, after = None, limit = 1000))]
fn kv_scan(
    py: Python<'_>,
    db_path: String,
    prefix: String,
    after: Option<String>,
    limit: usize,
) -> PyResult<Vec<(String, Py<PyBytes>)>> {
    open_default(&db_path)?.scan(py, &prefix, after.as_deref(), limit)
}

/// Atomically write or delete many records in one commit.
#[pyfunction]
fn kv_write_batch(db_path: String, changes: Vec<(String, Option<Vec<u8>>)>) -> PyResult<()> {
    open_default(&db_path)?.write_batch(changes)
}

/// Read-only consistency report as JSON. Does not repair damaged pages.
#[pyfunction]
fn doctor_text(db_path: String) -> PyResult<String> {
    open_default(&db_path)?.doctor()
}

/// Checkpoint, then copy a self-contained main file to `dest`.
#[pyfunction]
fn backup_db(db_path: String, dest: String) -> PyResult<()> {
    open_default(&db_path)?.backup(&dest)
}

/// Fold the WAL and reclaim safe orphan objects (`.yydx` layouts only).
#[pyfunction]
fn compact_db(db_path: String) -> PyResult<String> {
    open_default(&db_path)?.compact()
}

/// Open a verified backup copy (`.yydb` pages or `.yydx` object closure).
#[pyfunction]
fn restore(backup: String) -> PyResult<PyConnection> {
    PyConnection::restore_handle(&backup)
}

fn summarize_document(document: &str) -> String {
    let trimmed = document.trim();
    if trimmed.is_empty() {
        return "(empty)".to_owned();
    }
    const LIMIT: usize = 120;
    if trimmed.chars().count() <= LIMIT {
        return trimmed.replace('\n', " ");
    }
    let mut out: String = trimmed.chars().take(LIMIT).collect();
    out.push('…');
    out.replace('\n', " ")
}

#[pymodule]
fn yydb_pyo3(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(version, m)?)?;
    m.add_function(wrap_pyfunction!(serve, m)?)?;
    m.add_function(wrap_pyfunction!(init_db, m)?)?;
    m.add_function(wrap_pyfunction!(info_text, m)?)?;
    m.add_function(wrap_pyfunction!(read_schema_file, m)?)?;
    m.add_function(wrap_pyfunction!(query_vos, m)?)?;
    m.add_function(wrap_pyfunction!(execute_vos_write, m)?)?;
    m.add_function(wrap_pyfunction!(execute_vos_transaction, m)?)?;
    m.add_function(wrap_pyfunction!(put_file, m)?)?;
    m.add_function(wrap_pyfunction!(read_file_window, m)?)?;
    m.add_function(wrap_pyfunction!(read_file, m)?)?;
    m.add_class::<PyConnection>()?;
    m.add_function(wrap_pyfunction!(kv_put, m)?)?;
    m.add_function(wrap_pyfunction!(kv_get, m)?)?;
    m.add_function(wrap_pyfunction!(kv_delete, m)?)?;
    m.add_function(wrap_pyfunction!(kv_scan, m)?)?;
    m.add_function(wrap_pyfunction!(kv_write_batch, m)?)?;
    m.add_function(wrap_pyfunction!(doctor_text, m)?)?;
    m.add_function(wrap_pyfunction!(backup_db, m)?)?;
    m.add_function(wrap_pyfunction!(compact_db, m)?)?;
    m.add_function(wrap_pyfunction!(restore, m)?)?;
    m.add_class::<sqlite_bindings::SqliteWriter>()?;
    m.add_class::<sqlite_bindings::SqliteDatabase>()?;
    Ok(())
}
