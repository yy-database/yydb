//! fixture: format_v0.shm_sidecar

use std::fs;

use yydb::{journal::shm_path, Connection, OpenFlags};
use yydb_format::{parse_shm, committed_tail_lsn, parse_wal};

fn temp_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "yydb-format-v0-shm-sidecar-{}-{}.yydb",
        std::process::id(),
        label
    ))
}

#[test]
fn yydb_format_v0_shm_sidecar() {
    let path = temp_path("shm");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(shm_path(&path));

    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    conn.put("k", b"v").unwrap();
    drop(conn);

    let shm_bytes = fs::read(shm_path(&path)).unwrap();
    let shm = parse_shm(&shm_bytes).unwrap();
    assert_eq!(shm.format_version, 0);

    let wal_bytes = fs::read(path.with_extension("yydb-wal")).unwrap();
    let wal = parse_wal(&wal_bytes).unwrap();
    assert_eq!(shm.wal_tail_lsn, committed_tail_lsn(&wal));
    assert_eq!(shm.wal_file_bytes, wal_bytes.len() as u64);

    let reopened = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    reopened.checkpoint().unwrap();
    assert!(!shm_path(&path).exists());

    let _ = fs::remove_file(&path);
}
