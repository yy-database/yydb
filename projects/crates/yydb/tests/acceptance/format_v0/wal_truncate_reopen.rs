//! fixture: format_v0.wal_truncate_reopen

use std::fs;

use yydb::{journal::wal_path, Connection, OpenFlags};
use yydb_format::{
    parse_wal, strip_trailing_txn_commit, truncate_wal_to_recoverable_prefix, FilePager,
};

fn temp_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "yydb-format-v0-wal-truncate-{}-{}.yydb",
        std::process::id(),
        label
    ))
}

#[test]
fn yydb_format_v0_wal_truncate_reopen() {
    let path = temp_path("truncate-reopen");
    let wal = wal_path(&path);
    let _ = fs::remove_file(&wal);
    let _ = fs::remove_file(&path);

    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    conn.put("baseline", b"ok").unwrap();
    conn.checkpoint().unwrap();
    drop(conn);

    let mut pager = FilePager::open(&path, true).unwrap();
    pager.put_kv("lost", b"gone").unwrap();
    drop(pager);

    let wal_bytes = fs::read(&wal).unwrap();
    let without_commit = strip_trailing_txn_commit(&wal_bytes).unwrap();
    let mut torn_raw = without_commit.clone();
    torn_raw.extend_from_slice(&[0x02, 0xFF, 0xAA, 0x55, 0x00, 0x00, 0x00, 0x00]);
    assert!(
        parse_wal(&torn_raw).is_err(),
        "full WAL parse must reject torn tail bytes"
    );
    let recovered = truncate_wal_to_recoverable_prefix(&torn_raw).unwrap();
    assert_eq!(
        recovered,
        without_commit,
        "recoverable prefix must match the last committed WAL view"
    );
    fs::write(&wal, recovered).unwrap();

    let reopened = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    assert_eq!(reopened.get("baseline").unwrap(), Some(b"ok".to_vec()));
    assert_eq!(reopened.get("lost").unwrap(), None);

    let _ = fs::remove_file(&wal);
    let _ = fs::remove_file(&path);
}
