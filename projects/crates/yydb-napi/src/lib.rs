//! Node-API surface for the TypeScript `yydb` CLI and `@yydb/yydb` package.

use std::fs;
use std::path::Path;

use napi::bindgen_prelude::*;
use napi_derive::napi;

#[napi]
pub fn version() -> String {
    yydb::version().to_string()
}

#[napi]
pub fn serve(db_path: String, bind: String, insecure_bind: Option<bool>) -> Result<()> {
    yydb::serve::run_serve(
        Path::new(&db_path),
        &bind,
        insecure_bind.unwrap_or(false),
    )
    .map_err(|error| Error::from_reason(error.to_string()))?;
    Ok(())
}

#[napi]
pub fn init_db(
    db_path: String,
    schema_version: u32,
    schema_document: Option<String>,
) -> Result<()> {
    let conn = yydb::Connection::open(&db_path)
        .map_err(|error| Error::from_reason(error.to_string()))?;
    let schema = conn
        .schema()
        .map_err(|error| Error::from_reason(error.to_string()))?;
    let document = match schema_document {
        Some(document) => document,
        None if schema.is_none() => String::new(),
        None => return Ok(()),
    };
    conn.ensure_schema(schema_version, &document)
        .map_err(|error| Error::from_reason(error.to_string()))?;
    Ok(())
}

#[napi]
pub fn info_text(db_path: String) -> Result<String> {
    let conn = yydb::Connection::open(&db_path)
        .map_err(|error| Error::from_reason(error.to_string()))?;
    let path_display = conn
        .path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<memory>".to_owned());
    let mut lines = vec![format!("path={path_display}")];
    match conn
        .schema()
        .map_err(|error| Error::from_reason(error.to_string()))?
    {
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
        conn.record_count()
            .map_err(|error| Error::from_reason(error.to_string()))?
    ));
    lines.push(format!("journal_mode={}", conn.journal_mode().as_str()));
    if let (Some(wal), Some(shm)) = (conn.wal_path(), conn.shm_path()) {
        let (wal_on, shm_on, frames) = yydb::journal::sidecar_status(
            conn.path().expect("file-backed"),
        )
        .map_err(|error| Error::from_reason(error.to_string()))?;
        lines.push(format!("wal={} exists={wal_on}", wal.display()));
        lines.push(format!("shm={} exists={shm_on}", shm.display()));
        lines.push(format!("wal_frames={frames}"));
    }
    Ok(lines.join("\n"))
}

#[napi]
pub fn read_schema_file(schema_path: String) -> Result<String> {
    fs::read_to_string(&schema_path).map_err(|error| Error::from_reason(error.to_string()))
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
