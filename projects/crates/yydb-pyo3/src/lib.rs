//! Thin Python binding — delegates to the [`yydb`] facade and [`yydb-server`].
//!
//! TypeScript hosts use `yydb-napi`. Do not import `yydb-types` from Python glue.

use std::fs;
use std::path::Path;

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

fn map_err(error: impl ToString) -> PyErr {
    PyRuntimeError::new_err(error.to_string())
}

/// Embedded `yydb` engine version string.
#[pyfunction]
fn version() -> String {
    yydb::version().to_string()
}

/// Run `yydb-server` on `bind` for the database at `db_path`.
#[pyfunction]
#[pyo3(signature = (db_path, bind, insecure_bind=None))]
fn serve(db_path: String, bind: String, insecure_bind: Option<bool>) -> PyResult<()> {
    yydb_server::run_serve(
        Path::new(&db_path),
        &bind,
        insecure_bind.unwrap_or(false),
    )
    .map_err(map_err)?;
    Ok(())
}

/// Open `db_path` and ensure the VOS schema when needed.
#[pyfunction]
#[pyo3(signature = (db_path, schema_document=None))]
fn init_db(db_path: String, schema_document: Option<String>) -> PyResult<()> {
    let conn = yydb::Connection::open(&db_path).map_err(map_err)?;
    let schema = conn.schema().map_err(map_err)?;
    let document = match schema_document {
        Some(document) => document,
        None if schema.is_none() => String::new(),
        None => return Ok(()),
    };
    conn.ensure_schema(&document).map_err(map_err)?;
    Ok(())
}

/// Return multi-line database diagnostics for `db_path`.
#[pyfunction]
fn info_text(db_path: String) -> PyResult<String> {
    let conn = yydb::Connection::open(&db_path).map_err(map_err)?;
    let path_display = conn
        .path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<memory>".to_owned());
    let mut lines = vec![format!("path={path_display}")];
    match conn.schema().map_err(map_err)? {
        Some(schema) => {
            lines.push(format!("schema.version={}", schema.version));
            lines.push(format!(
                "schema.document={}",
                summarize_document(&schema.document)
            ));
        }
        None => lines.push("schema=(none)".to_owned()),
    }
    lines.push(format!(
        "records={}",
        conn.record_count().map_err(map_err)?
    ));
    lines.push(format!("journal_mode={}", conn.journal_mode().as_str()));
    if let (Some(wal), Some(shm)) = (conn.wal_path(), conn.shm_path()) {
        let (wal_on, shm_on, frames) =
            yydb::journal::sidecar_status(conn.path().expect("file-backed")).map_err(map_err)?;
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
    Ok(())
}
