//! In-process logical OPFS file tree for host-testable persistent I/O (Living `08` §3).
//!
//! Browser builds will replace this backend with OPFS sync-handle bindings. The logical
//! path layout matches local fs: `app.yydb`, `app.yydb-wal`, `<stem>-objects/<hash>.blob`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use yydb::{Error, Result};

fn logical_store() -> &'static Mutex<HashMap<String, Vec<u8>>> {
    static STORE: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn logical_key(db_path: &str, suffix: &str) -> String {
    format!("{db_path}{suffix}")
}

fn staging_key(db_path: &str, suffix: &str) -> String {
    format!("{db_path}{suffix}.staging")
}

/// Read a logical file under `db_path` (empty `suffix` is the main database file).
pub(crate) fn read_logical_file(db_path: &str, suffix: &str) -> Result<Option<Vec<u8>>> {
    let key = logical_key(db_path, suffix);
    Ok(logical_store()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&key)
        .cloned())
}

/// Atomically publish `body` as the visible logical file (staging → verify → swap).
pub(crate) fn publish_logical_file_atomic(db_path: &str, suffix: &str, body: &[u8]) -> Result<()> {
    let staging = staging_key(db_path, suffix);
    let visible = logical_key(db_path, suffix);
    let mut store = logical_store().lock().unwrap_or_else(|p| p.into_inner());
    store.insert(staging.clone(), body.to_vec());
    if let Some(existing) = store.get(&visible) {
        if existing == body {
            store.remove(&staging);
            return Ok(());
        }
    }
    store.insert(visible, body.to_vec());
    store.remove(&staging);
    Ok(())
}

/// Durability sync boundary for the logical database root (stub: in-memory flush).
pub(crate) fn sync_logical_root(db_path: &str) -> Result<()> {
    let _ = db_path;
    Ok(())
}

/// Blob path under `<stem>-objects/<hash>.blob` for `.yydx` layouts.
pub(crate) fn yydx_blob_key(db_path: &str, blob_hash: &str) -> String {
    let stem = db_path.strip_suffix(".yydx").unwrap_or(db_path);
    format!("{stem}-objects/{blob_hash}.blob")
}

/// Publish one immutable blob chunk with atomic no-replace semantics.
pub(crate) fn publish_blob_atomic(db_path: &str, blob_hash: &str, body: &[u8]) -> Result<()> {
    let key = yydx_blob_key(db_path, blob_hash);
    let mut store = logical_store().lock().unwrap_or_else(|p| p.into_inner());
    if let Some(existing) = store.get(&key) {
        if existing == body {
            return Ok(());
        }
        return Err(Error::CasConflict { key });
    }
    store.insert(key, body.to_vec());
    Ok(())
}

/// Read a published blob chunk, if present.
pub(crate) fn read_blob(db_path: &str, blob_hash: &str) -> Result<Option<Vec<u8>>> {
    let key = yydx_blob_key(db_path, blob_hash);
    Ok(logical_store()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&key)
        .cloned())
}
