// fixture: yydb.orphan.reclaim_idempotent

use crate::fixtures::yydb::{cleanup, open_temp_yydx};
use yydb::ObjectKind;

#[test]
fn yydb_orphan_reclaim_idempotent() {
    let (conn, path) = open_temp_yydx("orphan-reclaim-idempotent");
    let orphan = conn.put_chunk(ObjectKind::Blob, b"once").unwrap();

    let first = conn.reclaim_orphans(&[orphan.clone()]).unwrap();
    assert_eq!(first.reclaimed_objects, 1);

    let second = conn.reclaim_orphans(&[orphan.clone()]).unwrap();
    assert_eq!(
        second.reclaimed_objects,
        0,
        "reclaim must be idempotent when the chunk is already gone"
    );

    cleanup(&path);
}
