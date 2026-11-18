// `YYWL` / `YYSH` sidecars travel through the same host store as the main file.
//
// `.yydx` blob volumes are refused by this binding (see `crash_after_blob_publish`),
// so the sidecar contract is exercised on the supported single-file layout.

use std::path::PathBuf;

use crate::fixtures::{cleanup, fs_store, temp_db};
use database::execution::engine::Connection;
use yydb_wasm::PersistentVolume;

fn sidecar(path: &PathBuf, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn main_image() -> Vec<u8> {
    let conn = Connection::open_in_memory().expect("memory");
    conn.put("seed", b"value").expect("seed write");
    conn.main_snapshot().expect("main snapshot")
}

#[test]
fn yydb_opfs_wal_sidecar_roundtrip() {
    let path = temp_db("wal-sidecar");
    cleanup(&path);
    cleanup(&sidecar(&path, "-wal"));
    cleanup(&sidecar(&path, "-shm"));

    let volume = PersistentVolume::open(path.to_str().unwrap(), fs_store(&path)).expect("persistent open");
    volume.publish_main(&main_image()).expect("publish main");
    volume.publish_wal(b"YYWL-sidecar").expect("publish wal");
    volume.publish_shm(b"YYSH-sidecar").expect("publish shm");

    assert_eq!(volume.read_wal().expect("read wal"), Some(b"YYWL-sidecar".to_vec()));
    assert_eq!(volume.read_shm().expect("read shm"), Some(b"YYSH-sidecar".to_vec()));
    assert_eq!(std::fs::read(sidecar(&path, "-wal")).expect("wal sidecar file"), b"YYWL-sidecar");
    assert_eq!(std::fs::read(sidecar(&path, "-shm")).expect("shm sidecar file"), b"YYSH-sidecar");

    volume.remove_wal().expect("remove wal");
    volume.remove_shm().expect("remove shm");
    assert_eq!(volume.read_wal().expect("read wal"), None);
    assert_eq!(volume.read_shm().expect("read shm"), None);
    assert!(volume.read_main().expect("read main").is_some(), "removing sidecars must not touch the main file");

    cleanup(&path);
    cleanup(&sidecar(&path, "-wal"));
    cleanup(&sidecar(&path, "-shm"));
}
