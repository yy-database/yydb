//! gate: format-v0 (Living 09)
//! fixture: format_v0.wal_crash

use std::fs;

use yydb::{journal::wal_path, Connection, OpenFlags};
use yydb_format::{parse_wal, strip_trailing_txn_commit, FilePager};

fn temp_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "g-yydb-format-v0-wal-crash-{}-{}.yydb",
        std::process::id(),
        label
    ))
}

#[test]
fn g_yydb_format_v0_wal_crash() {
    let path = temp_path("crash");
    let wal = wal_path(&path);
    let _ = fs::remove_file(&wal);
    let _ = fs::remove_file(&path);

    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    conn.put("baseline", b"ok").unwrap();
    conn.checkpoint().unwrap();
    assert!(
        !wal.exists(),
        "checkpoint must remove the WAL sidecar before the crash txn"
    );
    drop(conn);

    let mut pager = FilePager::open(&path, true).unwrap();
    pager.put_kv("lost", b"gone").unwrap();
    drop(pager);

    let wal_bytes = fs::read(&wal).unwrap();
    let parsed = parse_wal(&wal_bytes).unwrap();
    assert_eq!(
        parsed
            .frames
            .iter()
            .filter(|frame| frame.frame_type == 0x04)
            .count(),
        1,
        "expected exactly one committed WAL txn before crash injection"
    );
    let crashed = strip_trailing_txn_commit(&wal_bytes).unwrap();
    assert!(
        parse_wal(&crashed)
            .unwrap()
            .frames
            .iter()
            .all(|frame| frame.frame_type != 0x04),
        "crash injection must drop TxnCommit"
    );
    fs::write(&wal, crashed).unwrap();

    let reopened = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    assert_eq!(reopened.get("baseline").unwrap(), Some(b"ok".to_vec()));
    assert_eq!(reopened.get("lost").unwrap(), None);

    let _ = fs::remove_file(&wal);
    let _ = fs::remove_file(&path);
}
