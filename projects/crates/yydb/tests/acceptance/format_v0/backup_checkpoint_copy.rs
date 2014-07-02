// fixture: format_v0.backup_checkpoint_copy

use crate::fixtures::yydb::{cleanup, temp_db};
use yydb::{journal::wal_path, Connection, OpenFlags};

#[test]
fn yydb_format_v0_backup_checkpoint_copy() {
    let path = temp_db("backup-checkpoint");
    let backup = temp_db("backup-checkpoint-copy");

    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).expect("open");
    conn.ensure_schema("table Setting { @@id: uuid, key: utf8, value: utf8 }")
        .expect("schema");
    conn.put("theme", b"dark").expect("put");
    assert!(wal_path(&path).exists(), "wal sidecar should exist before backup");

    conn.backup_to(&backup).expect("backup");
    assert!(
        !wal_path(&backup).exists(),
        "checkpoint backup must not copy wal sidecar"
    );
    drop(conn);

    let restored = Connection::open(&backup).expect("open backup");
    assert_eq!(
        restored.get("theme").expect("get"),
        Some(b"dark".to_vec())
    );
    let schema = restored.schema().expect("schema").expect("present");
    assert!(schema.document.contains("table Setting"));

    cleanup(&path);
    cleanup(&backup);
}

#[test]
fn yydb_format_v0_backup_with_wal_copy() {
    let path = temp_db("backup-wal");
    let backup = temp_db("backup-wal-copy");

    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).expect("open");
    conn.put("counter", b"1").expect("put");
    assert!(wal_path(&path).exists());

    conn.backup_with_wal_to(&backup).expect("backup with wal");
    assert!(
        wal_path(&backup).exists(),
        "wal-inclusive backup must copy sidecar"
    );
    drop(conn);

    let restored = Connection::open(&backup).expect("open backup");
    assert_eq!(
        restored.get("counter").expect("get"),
        Some(b"1".to_vec())
    );

    cleanup(&path);
    cleanup(&backup);
}
