//! Shared helpers for `g_yydb_*` gate fixtures.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use yydb::journal::{shm_path, wal_path};
use yydb::Connection;

pub fn temp_db_path(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("yydb-gate-{label}-{nonce}.yydb"))
}

pub fn open_temp_db(label: &str) -> (Connection, PathBuf) {
    let path = temp_db_path(label);
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
    let _ = fs::remove_file(lock_path(path));
}

fn lock_path(db: &Path) -> PathBuf {
    let mut sidecar = db.as_os_str().to_owned();
    sidecar.push("-lock");
    PathBuf::from(sidecar)
}
