//! fixture: format_v0.blob_publish_orphan_wal

use std::fs;
use std::path::{Path, PathBuf};

use yydb::{journal::wal_path, Batch, Connection, DoctorSeverity, ObjectKind, ObjectStore, OpenFlags};

fn temp_yydx(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "yydb-format-v0-blob-orphan-wal-{}-{}.yydx",
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
fn yydb_format_v0_blob_publish_orphan_wal() {
    let path = temp_yydx("publish-orphan-wal");
    cleanup(&path);

    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    let live = conn.put_chunk(ObjectKind::Blob, b"live-payload").unwrap();
    let mut batch = Batch::new();
    batch.attach_object(&live, "blobs/live");
    batch.commit(&conn).unwrap();
    conn.checkpoint().unwrap();

    let orphan = conn.put_chunk(ObjectKind::Blob, b"orphan-payload").unwrap();
    assert!(conn.objects().path_for(&orphan).exists());
    drop(conn);

    let reopened = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    assert_eq!(
        reopened.get("blobs/live").unwrap(),
        Some(format!("yydb:object:{}", live.hash_hex()).into_bytes())
    );
    assert_eq!(reopened.get("blobs/orphan").unwrap(), None);

    let orphans = reopened.scan_orphans("blobs/").unwrap();
    assert!(
        orphans
            .iter()
            .any(|object| object.hash_hex() == orphan.hash_hex()),
        "WAL mode must keep blob publish orphans off-catalog after reopen"
    );

    let report = reopened.doctor().unwrap();
    assert!(
        !report
            .issues
            .iter()
            .any(|issue| issue.severity == DoctorSeverity::Error),
        "doctor reported errors: {:?}",
        report.issues
    );

    cleanup(&path);
}
