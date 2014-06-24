//! fixture: format_v0.wal_blob_refs

use std::fs;
use std::path::{Path, PathBuf};

use yydb::{Batch, Connection, ObjectKind, ObjectStore, OpenFlags};
use yydb_format::{parse_wal, FRAME_BLOB_REFS};

fn temp_yydx(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "yydb-format-v0-wal-blob-refs-{}-{}.yydx",
        std::process::id(),
        label
    ))
}

fn cleanup(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(format!("{}-wal", path.display()));
    let _ = fs::remove_file(format!("{}-shm", path.display()));
    let _ = fs::remove_dir_all(ObjectStore::yydx_objects_root(path));
}

#[test]
fn yydb_format_v0_wal_blob_refs() {
    let path = temp_yydx("wal-blob-refs");
    cleanup(&path);

    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    let object = conn.put_chunk(ObjectKind::Blob, b"wal-blob-payload").unwrap();
    let mut batch = Batch::new();
    batch.attach_object(&object, "blobs/wal-live");
    batch.commit(&conn).unwrap();
    drop(conn);

    let wal_bytes = fs::read(format!("{}-wal", path.display())).unwrap();
    let wal = parse_wal(&wal_bytes).unwrap();
    let commit_index = wal
        .frames
        .iter()
        .rposition(|frame| frame.frame_type == 0x04)
        .expect("TxnCommit frame");
    let txn_begin_index = wal
        .frames
        .iter()
        .take(commit_index)
        .rposition(|frame| frame.frame_type == 0x01)
        .expect("TxnBegin frame");
    let txn_frames = &wal.frames[txn_begin_index..=commit_index];
    assert!(
        txn_frames
            .iter()
            .any(|frame| frame.frame_type == FRAME_BLOB_REFS),
        "WAL must record BlobRefs before TxnCommit for .yydx catalog attach"
    );
    let blob_refs_index = txn_frames
        .iter()
        .position(|frame| frame.frame_type == FRAME_BLOB_REFS)
        .expect("BlobRefs frame");
    let commit_in_txn = txn_frames.len() - 1;
    assert!(
        blob_refs_index < commit_in_txn,
        "BlobRefs must precede TxnCommit in the WAL txn"
    );

    let reopened = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    assert_eq!(
        reopened.get("blobs/wal-live").unwrap(),
        Some(format!("yydb:object:{}", object.hash_hex()).into_bytes())
    );
    assert_eq!(
        reopened.get_object(&object).unwrap().as_ref(),
        b"wal-blob-payload"
    );

    cleanup(&path);
}
