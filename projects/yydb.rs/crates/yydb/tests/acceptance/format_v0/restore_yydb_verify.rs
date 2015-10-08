// fixture: format_v0.restore_yydb_verify

use std::fs;

use crate::fixtures::yydb::{cleanup, temp_db, temp_db_with_ext};
use yydb::{Connection, Error, OpenFlags, journal::wal_path};

fn flip_byte(path: &std::path::Path, offset: usize) {
    let mut bytes = fs::read(path).expect("read copy");
    assert!(bytes.len() > offset + 4096, "backup must span past the corrupted page");
    bytes[offset] ^= 0x5A;
    fs::write(path, bytes).expect("write corrupted copy");
}

#[test]
fn yydb_formats_v0_restore_yydb_reads_checkpointed_backup() {
    let path = temp_db("restore-checkpoint");
    let backup = temp_db("restore-checkpoint-copy");

    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).expect("open");
    conn.ensure_schema("table Setting { @@id: uuid, key: utf8, value: utf8 }").expect("schema");
    conn.put("theme", b"dark").expect("put");
    conn.backup_to(&backup).expect("backup");
    assert!(!wal_path(&backup).exists(), "checkpoint backup must not copy wal sidecar");
    drop(conn);

    let restored = Connection::restore_yydb(&backup).expect("restore");
    assert_eq!(restored.get("theme").expect("get"), Some(b"dark".to_vec()));
    let schema = restored.schema().expect("schema").expect("present");
    assert!(schema.document.contains("table Setting"));
    restored.put("accent", b"green").expect("restored connection keeps serving writes");
    assert_eq!(restored.get("accent").expect("get"), Some(b"green".to_vec()));

    cleanup(&path);
    cleanup(&backup);
}

#[test]
fn yydb_formats_v0_restore_yydb_replays_wal_backup() {
    let path = temp_db("restore-wal");
    let backup = temp_db("restore-wal-copy");

    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).expect("open");
    conn.put("counter", b"41").expect("put");
    assert!(wal_path(&path).exists());
    conn.backup_with_wal_to(&backup).expect("backup with wal");
    assert!(wal_path(&backup).exists(), "wal-inclusive backup must copy sidecar");
    drop(conn);

    let restored = Connection::restore_yydb(&backup).expect("restore replays the copied wal");
    assert_eq!(restored.get("counter").expect("get"), Some(b"41".to_vec()));

    cleanup(&path);
    cleanup(&backup);
}

#[test]
fn yydb_formats_v0_restore_yydb_rejects_corrupt_data_page() {
    let path = temp_db("restore-corrupt-page");
    let backup = temp_db("restore-corrupt-page-copy");

    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).expect("open");
    conn.ensure_schema("table Setting { @@id: uuid, key: utf8, value: utf8 }").expect("schema");
    for index in 0..200 {
        conn.put(format!("key-{index}"), format!("value-{index}").as_bytes()).expect("put");
    }
    conn.backup_to(&backup).expect("backup");
    drop(conn);

    // Damage a payload byte inside the first data page; the CRC32C envelope no
    // longer matches and the restore must fail before open runs recovery.
    flip_byte(&backup, 4096 + 100);
    let error = Connection::restore_yydb(&backup).err().expect("corrupt copy must fail");
    assert!(matches!(error, Error::Corrupt(_)), "expected a corruption failure, got {error:?}");

    cleanup(&path);
    cleanup(&backup);
}

#[test]
fn yydb_formats_v0_restore_yydb_rejects_torn_copy() {
    let path = temp_db("restore-torn");
    let backup = temp_db("restore-torn-copy");

    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).expect("open");
    conn.put("theme", b"dark").expect("put");
    conn.backup_to(&backup).expect("backup");
    drop(conn);

    // A copy truncated mid-page is not a multiple of the page size.
    let bytes = fs::read(&backup).expect("read copy");
    fs::write(&backup, &bytes[..bytes.len() - 100]).expect("truncate copy");
    let error = Connection::restore_yydb(&backup).err().expect("torn copy must fail");
    assert!(matches!(error, Error::Corrupt(_)), "expected a corruption failure, got {error:?}");

    cleanup(&path);
    cleanup(&backup);
}

#[test]
fn yydb_formats_v0_restore_yydb_rejects_non_ydpg_bytes() {
    let backup = temp_db("restore-garbage");
    fs::write(&backup, b"not a yydb backup at all").expect("write garbage");

    let error = Connection::restore_yydb(&backup).err().expect("garbage bytes must fail");
    assert!(matches!(error, Error::Corrupt(_)), "expected a corruption failure, got {error:?}");

    cleanup(&backup);
}

#[test]
fn yydb_formats_v0_restore_yydb_rejects_yydx_path() {
    let backup = temp_db_with_ext("restore-wrong-layout", "yydx");

    let error = Connection::restore_yydb(&backup).err().expect("a .yydx path must be rejected");
    assert!(matches!(error, Error::Unsupported(_)), "expected an unsupported failure, got {error:?}");
}

#[test]
fn yydb_formats_v0_restore_yydb_missing_file_fails() {
    let backup = temp_db("restore-missing");
    assert!(Connection::restore_yydb(&backup).is_err(), "a missing backup file must fail");
}
