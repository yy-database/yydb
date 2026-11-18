//! Shared helpers for `yydb-wasm` integration and OPFS acceptance fixtures.

use std::path::PathBuf;

use database::storage::local_io::{FsBlobDirectory, FsVolumeStore, HostBlobDirectory, HostVolumeStore};
use serde_json::Value as JsonValue;

pub const USER_SCHEMA: &str = "table User { @@id: i64, name: utf8 }";

pub fn parse_json(payload: &str) -> JsonValue {
    serde_json::from_str(payload).expect("valid JSON envelope")
}

pub fn assert_ok_true(payload: &str) {
    let parsed = parse_json(payload);
    assert_eq!(parsed.get("ok"), Some(&JsonValue::Bool(true)));
}

/// Unique on-disk database path for one fixture.
pub fn temp_db(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("yydb-wasm-{name}-{}.yydb", std::process::id()))
}

/// Unique on-disk `.yydx` path for one fixture.
pub fn temp_yydx(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("yydb-wasm-{name}-{}.yydx", std::process::id()))
}

/// Byte store over the real file system, the host equivalent of OPFS handles.
pub fn fs_store(path: &PathBuf) -> Box<dyn HostVolumeStore> {
    Box::new(FsVolumeStore::open(path))
}

/// `.yydx` sibling object directory over the real file system.
pub fn fs_blob_directory(path: &PathBuf) -> Box<dyn HostBlobDirectory> {
    Box::new(FsBlobDirectory::open(path))
}

pub fn cleanup(path: &PathBuf) {
    let _ = std::fs::remove_file(path);
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(sidecar));
    }
    if let Some(parent) = path.parent() {
        let stem = path.file_stem().and_then(|name| name.to_str()).unwrap_or("app");
        let _ = std::fs::remove_dir_all(parent.join(format!("{stem}-objects")));
    }
}
