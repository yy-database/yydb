//! Shared helpers for acceptance fixture harness (`open_temp_db`, temp paths, cleanup).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use yydb::journal::{shm_path, wal_path};
use yydb::{Connection, ObjectStore};

pub fn temp_db_with_ext(label: &str, ext: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("yydb-{label}-{nonce}.{ext}"))
}

pub fn temp_db(label: &str) -> PathBuf {
    temp_db_with_ext(label, "yydb")
}

pub fn temp_yydx(label: &str) -> PathBuf {
    temp_db_with_ext(label, "yydx")
}

pub fn temp_db_path(label: &str) -> PathBuf {
    temp_db(label)
}

pub fn open_temp_db(label: &str) -> (Connection, PathBuf) {
    let path = temp_db(label);
    let conn = Connection::open(&path).unwrap();
    (conn, path)
}

pub fn reopen(path: &Path) -> Connection {
    Connection::open(path).unwrap()
}

pub fn objects_sidecar(path: &Path) -> PathBuf {
    let mut sidecar = path.as_os_str().to_owned();
    sidecar.push(".objects");
    PathBuf::from(sidecar)
}

pub fn cleanup(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(wal_path(path));
    let _ = fs::remove_file(shm_path(path));
    let _ = fs::remove_file(lock_path(path));
    let _ = fs::remove_dir_all(objects_sidecar(path));
    let _ = fs::remove_dir_all(ObjectStore::yydx_objects_root(path));
}

fn lock_path(db: &Path) -> PathBuf {
    let mut sidecar = db.as_os_str().to_owned();
    sidecar.push("-lock");
    PathBuf::from(sidecar)
}
