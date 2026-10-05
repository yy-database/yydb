//! fixture: format_v0.blob_publish_orphan

use std::fs;
use std::path::{Path, PathBuf};

use yydb::{Batch, Connection, DoctorSeverity, ObjectKind, ObjectStore};

fn temp_yydx(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "yydb-format-v0-blob-orphan-{}-{}.yydx",
        std::process::id(),
        label
    ))
}

fn cleanup(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(format!("{}-wal", path.display()));
    let _ = fs::remove_dir_all(ObjectStore::yydx_objects_root(path));
}

#[test]
fn yydb_format_v0_blob_publish_orphan() {
    let path = temp_yydx("publish-orphan");
    cleanup(&path);

    let conn = Connection::open_yydx(&path).unwrap();
    let live = conn.put_chunk(ObjectKind::Blob, b"live-payload").unwrap();
    let mut batch = Batch::new();
    batch.attach_object(&live, "blobs/live");
    batch.commit(&conn).unwrap();

    // Crash after blob publish (step 3) but before catalog commit (steps 4–6).
    let orphan = conn.put_chunk(ObjectKind::Blob, b"orphan-payload").unwrap();
    assert!(conn.objects().path_for(&orphan).exists());
    drop(conn);

    let reopened = Connection::open_yydx(&path).unwrap();
    assert_eq!(
        reopened.get("blobs/live").unwrap(),
        Some(format!("yydb:object:{}", live.hash_hex()).into_bytes())
    );
    assert_eq!(
        reopened.get_object(&live).unwrap().as_ref(),
        b"live-payload"
    );
    assert_eq!(reopened.get("blobs/orphan").unwrap(), None);

    let orphans = reopened.scan_orphans("blobs/").unwrap();
    assert!(
        orphans
            .iter()
            .any(|object| object.hash_hex() == orphan.hash_hex()),
        "published orphan must remain off-catalog after reopen"
    );

    let report = reopened.doctor().unwrap();
    assert!(
        report
            .issues
            .iter()
            .all(|issue| issue.code != "yydb.doctor.missing_blob"),
        "committed catalog must not reference unpublished blobs: {:?}",
        report.issues
    );
    assert!(
        !report
            .issues
            .iter()
            .any(|issue| issue.severity == DoctorSeverity::Error),
        "doctor reported errors: {:?}",
        report.issues
    );
    assert!(report.orphan_object_count >= 1);

    let reclaim = reopened.reclaim_orphans(&[orphan.clone()]).unwrap();
    assert_eq!(reclaim.reclaimed_objects, 1);
    assert!(reopened.get_object(&live).is_ok());
    assert!(!reopened.objects().path_for(&orphan).exists());

    cleanup(&path);
}
