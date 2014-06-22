//! Connection facade over `YDPG` format v0.

use std::fs;

use yydb::{Connection, OpenFlags};
use yydb_format::PAGE_MAGIC;

fn temp_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "yydb-format-v0-connection-{}-{}.yydb",
        std::process::id(),
        label
    ))
}

#[test]
fn yydb_format_v0_connection_put_get_reopen() {
    let path = temp_path("roundtrip");
    let _ = fs::remove_file(&path);

    let conn = Connection::open_with_flags(&path, OpenFlags::new()).unwrap();
    conn.put("alpha", b"one").unwrap();
    conn.put("beta", b"two").unwrap();
    drop(conn);

    let bytes = fs::read(&path).unwrap();
    assert_eq!(bytes.get(0..5), Some(PAGE_MAGIC.as_slice()));

    let reopened = Connection::open(&path).unwrap();
    assert_eq!(reopened.get("alpha").unwrap(), Some(b"one".to_vec()));
    assert_eq!(reopened.get("beta").unwrap(), Some(b"two".to_vec()));
    let _ = fs::remove_file(&path);
}

#[test]
fn yydb_format_v0_connection_wal_checkpoint() {
    let path = temp_path("wal");
    let _ = fs::remove_file(&path);
    let wal = std::path::PathBuf::from(format!("{}-wal", path.display()));
    let _ = fs::remove_file(&wal);

    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    conn.put("k", b"v").unwrap();
    assert!(conn.wal_path().unwrap().exists());
    conn.checkpoint().unwrap();
    assert!(!conn.wal_path().unwrap().exists());
    drop(conn);

    let reopened = Connection::open(&path).unwrap();
    assert_eq!(reopened.get("k").unwrap(), Some(b"v".to_vec()));
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(wal);
}
