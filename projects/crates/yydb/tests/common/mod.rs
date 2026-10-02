//! Shared helpers for `g_yydb_*` gate fixtures.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use yydb::journal::{shm_path, wal_path};
use yydb::Connection;

pub fn open_temp_db(label: &str) -> (Connection, PathBuf) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("yydb-gate-{label}-{nonce}.yydb"));
    let conn = Connection::open(&path).unwrap();
    (conn, path)
}

pub fn reopen(path: &Path) -> Connection {
    Connection::open(path).unwrap()
}

pub fn cleanup(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(wal_path(path));
    let _ = fs::remove_file(shm_path(path));
}
