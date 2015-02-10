// fixture: yydb.orphan.doctor_count

use crate::fixtures::yydb::{cleanup, open_temp_yydx};
use yydb::{Batch, ObjectKind};

#[test]
fn yydb_orphan_doctor_count() {
    let (conn, path) = open_temp_yydx("orphan-doctor-count");

    let live = conn.put_chunk(ObjectKind::Blob, b"keep").unwrap();
    let mut batch = Batch::new();
    batch.attach_object(&live, "pages/kept");
    batch.commit(&conn).unwrap();

    conn.put_chunk(ObjectKind::Blob, b"drop-a").unwrap();
    conn.put_chunk(ObjectKind::Blob, b"drop-b").unwrap();

    let before = conn.doctor().unwrap().orphan_object_count;
    assert_eq!(before, 2, "doctor must count unreferenced on-disk orphans");

    let orphans = conn.scan_orphans("pages/").unwrap();
    assert_eq!(orphans.len(), 2);
    let report = conn.reclaim_orphans(&orphans).unwrap();
    assert_eq!(report.reclaimed_objects, 2);

    let after = conn.doctor().unwrap().orphan_object_count;
    assert_eq!(after, 0, "doctor orphan count must drop to zero after reclaim");
    assert!(conn.get_object(&live).is_ok());

    cleanup(&path);
}
