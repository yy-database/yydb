// fixture: yydb.orphan.reclaim_skips_referenced

use crate::fixtures::yydb::{cleanup, open_temp_db};
use yydb::{Batch, ObjectKind};

#[test]
fn yydb_orphan_reclaim_skips_referenced() {
    let (conn, path) = open_temp_db("orphan-reclaim-skips-referenced");
    let live = conn.put_chunk(ObjectKind::Blob, b"keep").unwrap();
    let mut batch = Batch::new();
    batch.attach_object(&live, "pages/kept");
    batch.commit(&conn).unwrap();

    let orphan = conn.put_chunk(ObjectKind::Blob, b"drop").unwrap();
    let report = conn
        .reclaim_orphans(&[live.clone(), orphan.clone()])
        .unwrap();
    assert_eq!(
        report.reclaimed_objects,
        1,
        "reclaim must skip catalog-referenced objects"
    );
    assert!(conn.get_object(&live).is_ok());
    assert!(conn.get_object(&orphan).is_err());

    cleanup(&path);
}
