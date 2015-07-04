//! Thin Python binding — delegates to the [`yydb`] facade.
//!
//! TypeScript hosts use `yydb-napi`. Do not import `yydb-types` from Python glue.

#![deny(missing_docs)]

use std::{collections::BTreeMap, fs, path::Path};

use pyo3::{
    IntoPyObjectExt,
    exceptions::{PyRuntimeError, PyTypeError},
    prelude::*,
    types::{PyBool, PyBytes, PyDict, PyString},
};
use yydb::Value;

fn map_err(error: impl ToString) -> PyErr {
    PyRuntimeError::new_err(error.to_string())
}

/// Host value accepted by [`execute_vos_write_with_params`].
///
/// `bytes` stays `bytes` in both directions: it is never decoded as text and
/// never re-encoded, so a payload that no text spelling can carry survives the
/// host boundary unchanged. A tagged dict carrying `"$yydb": "file"` is
/// decoded into a [`yydb::Value::File`] descriptor so VOS `file` columns can
/// be round-tripped through the Python host.
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
        if dict.contains("$yydb")? {
            let json = dict_to_json(dict)?;
            if let Some(result) = yydb::types::scalar::tagged_value_from_json(&json) {
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
        Value::File(descriptor) => {
            let json = yydb::types::scalar::tagged_value_json(&Value::File(descriptor))
                .expect("`Value::File` always has a tagged form");
            json_to_py(py, &json)
        }
        other => Err(PyTypeError::new_err(format!("unsupported result value `{}`", other.kind()))),
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
    yydb::version().to_string()
}

/// Run the YYDB serve loop on `bind` for the database at `db_path`.
#[pyfunction]
#[pyo3(signature = (db_path, bind, insecure_bind=None))]
fn serve(db_path: String, bind: String, insecure_bind: Option<bool>) -> PyResult<()> {
    yydb::server::run_serve(Path::new(&db_path), &bind, insecure_bind.unwrap_or(false)).map_err(map_err)?;
    Ok(())
}

/// Open `db_path` and, when a document is given, ensure the VOS schema.
///
/// A new database stays schema-less until a document is provided, so callers
/// can open a handle first and apply the schema afterwards.
#[pyfunction]
#[pyo3(signature = (db_path, schema_document=None))]
fn init_db(db_path: String, schema_document: Option<String>) -> PyResult<()> {
    let conn = yydb::Connection::open(&db_path).map_err(map_err)?;
    conn.schema().map_err(map_err)?;
    if let Some(document) = schema_document {
        conn.ensure_schema(&document).map_err(map_err)?;
    }
    Ok(())
}

/// Return multi-line database diagnostics for `db_path`.
#[pyfunction]
fn info_text(db_path: String) -> PyResult<String> {
    let conn = yydb::Connection::open(&db_path).map_err(map_err)?;
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
        let (wal_on, shm_on, frames) = yydb::journal::sidecar_status(conn.path().expect("file-backed")).map_err(map_err)?;
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
    let conn = yydb::Connection::open(&db_path).map_err(map_err)?;
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
/// never pass through a text spelling. A dict with `"$yydb": "file"` is
/// accepted as a `Value::File` descriptor for VOS `file` columns.
#[pyfunction]
fn execute_vos_write(db_path: String, source: String, params: &Bound<'_, PyDict>) -> PyResult<()> {
    let conn = yydb::Connection::open(&db_path).map_err(map_err)?;
    let mut named = BTreeMap::new();
    for (key, value) in params.iter() {
        named.insert(key.extract::<String>()?, py_to_value(&value)?);
    }
    conn.execute_vos_write_with_params(&source, &named).map_err(map_err)
}

fn decode_descriptor_dict(dict: &Bound<'_, PyDict>) -> PyResult<yydb::FileDescriptor> {
    let json = dict_to_json(dict)?;
    let decoded = yydb::types::scalar::tagged_value_from_json(&json)
        .ok_or_else(|| PyRuntimeError::new_err("dict does not carry a `$yydb` tagged value"))?
        .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
    match decoded {
        Value::File(descriptor) => Ok(descriptor),
        other => Err(PyRuntimeError::new_err(format!(
            "expected `$yydb:file`, got `{}`",
            other.kind()
        ))),
    }
}

/// Publish a logical `file` value in a `.yydx` layout and return the
/// descriptor as a tagged `{"$yydb":"file",…}` dict.
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
    let conn = yydb::Connection::open(&db_path).map_err(map_err)?;
    let chunk_size = usize::try_from(chunk_size)
        .map_err(|_| PyRuntimeError::new_err("chunk_size exceeds platform pointer width"))?;
    let descriptor = conn
        .put_file(std::io::Cursor::new(payload), chunk_size, name, media_type)
        .map_err(map_err)?;
    let json = yydb::types::scalar::tagged_value_json(&Value::File(descriptor))
        .expect("`Value::File` always has a tagged form");
    json_to_py(py, &json)
}

/// Read one VOS window of a committed `file` value.
///
/// `descriptor` is a tagged `{"$yydb":"file",…}` dict — the shape produced by
/// [`put_file`] or queried from a VOS `file` column.
#[pyfunction]
#[pyo3(signature = (db_path, descriptor, offset, length))]
fn read_file_window(
    py: Python<'_>,
    db_path: String,
    descriptor: &Bound<'_, PyDict>,
    offset: i64,
    length: i64,
) -> PyResult<Py<PyBytes>> {
    let conn = yydb::Connection::open(&db_path).map_err(map_err)?;
    let descriptor = decode_descriptor_dict(descriptor)?;
    let offset = u64::try_from(offset).map_err(|_| PyRuntimeError::new_err("offset must be non-negative"))?;
    let length = u64::try_from(length).map_err(|_| PyRuntimeError::new_err("length must be non-negative"))?;
    let octets = conn
        .read_file_window(&descriptor, yydb::vos::types::FileReadWindow::new(offset, length))
        .map_err(map_err)?;
    Ok(PyBytes::new(py, &octets).into())
}

/// Read the complete octets of a committed `file` value and verify its
/// content digest.
#[pyfunction]
fn read_file(py: Python<'_>, db_path: String, descriptor: &Bound<'_, PyDict>) -> PyResult<Py<PyBytes>> {
    let conn = yydb::Connection::open(&db_path).map_err(map_err)?;
    let descriptor = decode_descriptor_dict(descriptor)?;
    let octets = conn.read_file(&descriptor).map_err(map_err)?;
    Ok(PyBytes::new(py, &octets).into())
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
    m.add_function(wrap_pyfunction!(put_file, m)?)?;
    m.add_function(wrap_pyfunction!(read_file_window, m)?)?;
    m.add_function(wrap_pyfunction!(read_file, m)?)?;
    Ok(())
}
