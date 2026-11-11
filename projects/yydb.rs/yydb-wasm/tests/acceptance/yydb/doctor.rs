use database::execution::engine::{Batch, Connection, DoctorSeverity, ObjectKind, OpenFlags};

use super::TempDb;

// fixture: yydb.wal.reopen_doctor
#[test]
fn yydb_wal_reopen_doctor() {
    let dir = TempDb::new("wal-reopen-doctor");
    let path = dir.yydb();
    let wal = path.with_extension("yydb-wal");
    {
        let conn = Connection::open_with_flags(&path, OpenFlags::wal()).expect("open yydb in wal mode");
        conn.put("wal/a", b"1").expect("put a");
        conn.put("wal/b", b"2").expect("put b");
        assert!(wal.exists(), "wal sidecar must exist after a commit in wal mode");
        conn.close();
    }

    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).expect("reopen yydb in wal mode");
    conn.checkpoint().expect("checkpoint");
    assert_eq!(conn.get("wal/a").expect("get a"), Some(b"1".to_vec()));
    assert_eq!(conn.get("wal/b").expect("get b"), Some(b"2".to_vec()));
    let report = conn.doctor().expect("doctor");
    assert!(
        !report.issues.iter().any(|issue| issue.severity == DoctorSeverity::Error),
        "doctor reported errors after wal reopen: {:?}",
        report.issues
    );
}

// fixture: yydb.orphan.reclaim_safe
#[test]
fn yydb_orphan_reclaim_safe() {
    let dir = TempDb::new("orphan-reclaim-safe");
    let conn = Connection::open(dir.yydb()).expect("open yydb");
    let live = conn.put_chunk(ObjectKind::Blob, b"live").expect("store live");
    let orphan = conn.put_chunk(ObjectKind::Blob, b"orphan").expect("store orphan");
    let mut batch = Batch::new();
    batch.attach_object(&live, "pages/live");
    batch.commit(&conn).expect("commit live attachment");

    let candidates = conn.scan_orphans("pages/").expect("scan orphans before reclaim");
    assert!(
        candidates.iter().any(|object| object.hash_hex() == orphan.hash_hex()),
        "unreferenced object must be scanned as an orphan"
    );

    let report = conn.reclaim_orphans(std::slice::from_ref(&orphan)).expect("reclaim orphan");
    assert_eq!(report.reclaimed_objects, 1);
    assert_eq!(&conn.get_object(&live).expect("live object survives")[..], b"live");
    assert!(conn.get_object(&orphan).is_err(), "reclaimed object must be gone");
}

// fixture: yydb.orphan.reclaim_skips_referenced
#[test]
fn yydb_orphan_reclaim_skips_referenced() {
    let dir = TempDb::new("orphan-reclaim-skips-referenced");
    let conn = Connection::open(dir.yydb()).expect("open yydb");
    let live = conn.put_chunk(ObjectKind::Blob, b"live").expect("store live");
    let orphan = conn.put_chunk(ObjectKind::Blob, b"orphan").expect("store orphan");
    let mut batch = Batch::new();
    batch.attach_object(&live, "pages/live");
    batch.commit(&conn).expect("commit live attachment");

    let report = conn.reclaim_orphans(&[live.clone(), orphan.clone()]).expect("reclaim candidates");
    assert_eq!(report.reclaimed_objects, 1, "only the unreferenced object may be reclaimed");
    assert_eq!(&conn.get_object(&live).expect("referenced object survives")[..], b"live");
    assert!(conn.get_object(&orphan).is_err());
}

// fixture: yydb.orphan.reclaim_skips_pinned
#[test]
fn yydb_orphan_reclaim_skips_pinned() {
    let dir = TempDb::new("orphan-reclaim-skips-pinned");
    let conn = Connection::open(dir.yydb()).expect("open yydb");
    let pinned = conn.put_chunk(ObjectKind::Blob, b"pinned").expect("store pinned");
    conn.pin_object(&pinned).expect("pin object");

    let first = conn.reclaim_orphans(std::slice::from_ref(&pinned)).expect("reclaim pinned");
    assert_eq!(first.reclaimed_objects, 0, "pinned objects must not be reclaimed");
    assert_eq!(&conn.get_object(&pinned).expect("pinned object readable")[..], b"pinned");

    conn.evict_object(&pinned).expect("unpin object");
    let second = conn.reclaim_orphans(std::slice::from_ref(&pinned)).expect("reclaim unpinned");
    assert_eq!(second.reclaimed_objects, 1);
    assert!(conn.get_object(&pinned).is_err());
}

// fixture: yydb.orphan.reclaim_idempotent
#[test]
fn yydb_orphan_reclaim_idempotent() {
    let dir = TempDb::new("orphan-reclaim-idempotent");
    let path = dir.yydx();
    {
        let conn = Connection::open_yydx(&path).expect("open yydx");
        let orphan = conn.put_chunk(ObjectKind::Blob, b"orphan").expect("store orphan");
        let first = conn.reclaim_orphans(std::slice::from_ref(&orphan)).expect("first reclaim");
        let second = conn.reclaim_orphans(std::slice::from_ref(&orphan)).expect("second reclaim");
        assert_eq!(first.reclaimed_objects, 1);
        assert_eq!(second.reclaimed_objects, 0, "reclaim must be idempotent");
        conn.close();
    }

    let conn = Connection::open_yydx(&path).expect("reopen yydx");
    assert!(conn.scan_orphans("").expect("scan after reopen").is_empty());
}

// fixture: yydb.orphan.doctor_count
#[test]
fn yydb_orphan_doctor_count() {
    let dir = TempDb::new("orphan-doctor-count");
    let conn = Connection::open_yydx(dir.yydx()).expect("open yydx");
    let live = conn.put_chunk(ObjectKind::Blob, b"live").expect("store live");
    let mut batch = Batch::new();
    batch.attach_object(&live, "assets/live");
    batch.commit(&conn).expect("commit live attachment");
    let first = conn.put_chunk(ObjectKind::Blob, b"orphan-one").expect("store orphan one");
    let second = conn.put_chunk(ObjectKind::Blob, b"orphan-two").expect("store orphan two");

    let candidates = conn.scan_orphans("").expect("scan orphans");
    assert_eq!(candidates.len(), 2, "doctor must count exactly the unreferenced objects");
    assert!(candidates.iter().any(|object| object.hash_hex() == first.hash_hex()));
    assert!(candidates.iter().any(|object| object.hash_hex() == second.hash_hex()));
    assert_eq!(conn.doctor().expect("doctor").orphan_object_count, 2, "doctor orphan count must match the scan");

    let report = conn.reclaim_orphans(&candidates).expect("reclaim all orphans");
    assert_eq!(report.reclaimed_objects, 2);
    let doctor = conn.doctor().expect("doctor");
    assert_eq!(doctor.orphan_object_count, 0, "doctor orphan count must drop to zero after reclaim");
    assert_eq!(doctor.issues.iter().filter(|issue| issue.severity == DoctorSeverity::Error).count(), 0);
    assert_eq!(&conn.get_object(&live).expect("live object survives")[..], b"live");
}
