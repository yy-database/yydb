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
/// host boundary unchanged.
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
    if let Ok(text) = value.cast::<PyString>() {
        return Ok(Value::Text(text.to_str()?.to_owned()));
    }
    if let Ok(number) = value.extract::<i64>() {
        return Ok(Value::I64(number));
    }
    Err(PyTypeError::new_err(format!("unsupported parameter value {value:?}")))
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
        other => Err(PyTypeError::new_err(format!("unsupported result value `{}`", other.kind()))),
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
/// never pass through a text spelling.
#[pyfunction]
fn execute_vos_write(db_path: String, source: String, params: &Bound<'_, PyDict>) -> PyResult<()> {
    let conn = yydb::Connection::open(&db_path).map_err(map_err)?;
    let mut named = BTreeMap::new();
    for (key, value) in params.iter() {
        named.insert(key.extract::<String>()?, py_to_value(&value)?);
    }
    conn.execute_vos_write_with_params(&source, &named).map_err(map_err)
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
    Ok(())
}
