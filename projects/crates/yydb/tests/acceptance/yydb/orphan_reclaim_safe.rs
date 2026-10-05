// fixture: yydb.orphan.reclaim_safe

#[path = "../../fixtures/yydb/mod.rs"]
mod harness;
use harness::{cleanup, open_temp_db};
use yydb::{Batch, ObjectKind};

#[test]
fn yydb_orphan_reclaim_safe() {
    let (conn, path) = open_temp_db("orphan-reclaim-safe");
    let live = conn.put_chunk(ObjectKind::Blob, b"keep").unwrap();
    let mut batch = Batch::new();
    batch.attach_object(&live, "pages/kept");
    batch.commit(&conn).unwrap();

    let orphan = conn.put_chunk(ObjectKind::Blob, b"drop").unwrap();
    let orphans = conn.scan_orphans("pages/").unwrap();
    assert!(
        orphans
            .iter()
            .any(|object| object.hash_hex() == orphan.hash_hex()),
        "fixture yydb.orphan.reclaim_safe expected orphan object"
    );

    let report = conn.reclaim_orphans(&[orphan.clone()]).unwrap();
    assert_eq!(report.reclaimed_objects, 1);
    assert!(conn.get_object(&live).is_ok());
    assert!(conn.get_object(&orphan).is_err());
    cleanup(&path);
}
