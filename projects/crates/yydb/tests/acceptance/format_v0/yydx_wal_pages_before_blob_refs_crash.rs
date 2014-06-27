//! fixture: format_v0.yydx_wal_pages_before_blob_refs_crash

use std::fs;
use std::path::{Path, PathBuf};

use yydb::{journal::wal_path, Batch, Connection, ObjectKind, ObjectStore, OpenFlags};
use yydb_format::{parse_wal, strip_incomplete_txn_after_last_page_image, FRAME_BLOB_REFS};

fn temp_yydx(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "yydb-format-v0-yydx-pages-before-refs-{}-{}.yydx",
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
fn yydb_format_v0_yydx_wal_pages_before_blob_refs_crash() {
    let path = temp_yydx("pages-before-refs");
    cleanup(&path);

    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    let baseline = conn.put_chunk(ObjectKind::Blob, b"baseline").unwrap();
    let mut batch = Batch::new();
    batch.attach_object(&baseline, "blobs/baseline");
    batch.commit(&conn).unwrap();
    conn.checkpoint().unwrap();
    drop(conn);

    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    let orphan = conn.put_chunk(ObjectKind::Blob, b"orphan-payload").unwrap();
    let mut batch = Batch::new();
    batch.attach_object(&orphan, "blobs/orphan");
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
        "committed txn must include BlobRefs before crash injection"
    );
    let crashed = strip_incomplete_txn_after_last_page_image(&wal_bytes).unwrap();
    assert!(
        !parse_wal(&crashed)
            .unwrap()
            .frames
            .iter()
            .any(|frame| frame.frame_type == FRAME_BLOB_REFS),
        "crash injection must drop BlobRefs and TxnCommit"
    );
    fs::write(&wal, crashed).unwrap();

    let reopened = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    assert_eq!(
        reopened.get("blobs/baseline").unwrap(),
        Some(format!("yydb:object:{}", baseline.hash_hex()).into_bytes())
    );
    assert_eq!(reopened.get("blobs/orphan").unwrap(), None);
    assert!(reopened.objects().path_for(&orphan).exists());

    cleanup(&path);
}
