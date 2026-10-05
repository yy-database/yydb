// fixture: yydb.orphan.reclaim_skips_pinned

use crate::fixtures::yydb::{cleanup, open_temp_db};
use yydb::ObjectKind;

#[test]
fn yydb_orphan_reclaim_skips_pinned() {
    let (conn, path) = open_temp_db("orphan-reclaim-skips-pinned");
    let orphan = conn.put_chunk(ObjectKind::Blob, b"pinned-orphan").unwrap();
    conn.pin_object(&orphan).unwrap();

    let report = conn.reclaim_orphans(&[orphan.clone()]).unwrap();
    assert_eq!(
        report.reclaimed_objects,
        0,
        "reclaim must skip pinned orphan objects"
    );
    assert!(conn.get_object(&orphan).is_ok());

    conn.evict_object(&orphan).unwrap();
    let report = conn.reclaim_orphans(&[orphan.clone()]).unwrap();
    assert_eq!(report.reclaimed_objects, 1);
    assert!(conn.get_object(&orphan).is_err());

    cleanup(&path);
}
