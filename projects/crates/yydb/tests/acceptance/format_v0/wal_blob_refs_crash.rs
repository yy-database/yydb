//! fixture: format_v0.wal_blob_refs_crash

use std::fs;
use std::path::{Path, PathBuf};

use yydb::{journal::wal_path, Batch, Connection, ObjectKind, ObjectStore, OpenFlags};
use yydb_format::{parse_wal, strip_trailing_txn_commit, FRAME_BLOB_REFS};

fn temp_yydx(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "yydb-format-v0-wal-blob-refs-crash-{}-{}.yydx",
        std::process::id(),
        label
    ))
}

fn cleanup(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(wal_path(path));
    let _ = fs::remove_file(format!("{}-shm", path.display()));
    let _ = fs::remove_dir_all(ObjectStore::yydx_objects_root(path));
}

#[test]
fn yydb_format_v0_wal_blob_refs_crash() {
    let path = temp_yydx("wal-blob-refs-crash");
    cleanup(&path);

    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    let baseline = conn.put_chunk(ObjectKind::Blob, b"baseline").unwrap();
    let mut batch = Batch::new();
    batch.attach_object(&baseline, "blobs/baseline");
    batch.commit(&conn).unwrap();
    conn.checkpoint().unwrap();
    drop(conn);

    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    let lost = conn.put_chunk(ObjectKind::Blob, b"lost").unwrap();
    let mut batch = Batch::new();
    batch.attach_object(&lost, "blobs/lost");
    batch.commit(&conn).unwrap();
    drop(conn);

    let wal = wal_path(&path);
    let wal_bytes = fs::read(&wal).unwrap();
    let parsed = parse_wal(&wal_bytes).unwrap();
    assert!(
        parsed
            .frames
            .iter()
            .any(|frame| frame.frame_type == FRAME_BLOB_REFS),
        "crash txn must include BlobRefs"
    );
    let crashed = strip_trailing_txn_commit(&wal_bytes).unwrap();
    assert!(
        parse_wal(&crashed)
            .unwrap()
            .frames
            .iter()
            .all(|frame| frame.frame_type != 0x04),
        "crash injection must drop TxnCommit after BlobRefs"
    );
    fs::write(&wal, crashed).unwrap();

    let reopened = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    assert_eq!(
        reopened.get("blobs/baseline").unwrap(),
        Some(format!("yydb:object:{}", baseline.hash_hex()).into_bytes())
    );
    assert_eq!(reopened.get("blobs/lost").unwrap(), None);
    assert!(reopened.objects().path_for(&lost).exists());

    cleanup(&path);
}
