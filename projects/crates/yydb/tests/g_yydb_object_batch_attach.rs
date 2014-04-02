// gate: G-YYDB-6
// fixture: yydb.object.batch_attach

mod common;

use common::{cleanup, open_temp_db};
use yydb::{Batch, ObjectKind};

#[test]
fn g_yydb_object_batch_attach() {
    let (conn, path) = open_temp_db("object-batch-attach");
    let live = conn.put_chunk(ObjectKind::Blob, b"live-payload").unwrap();

    let mut committed = Batch::new();
    committed.attach_object(&live, "pages/live");
    committed.commit(&conn).unwrap();
    assert_eq!(
        conn.get("pages/live").unwrap(),
        Some(format!("yydb:object:{}", live.hash_hex()).into_bytes())
    );
    assert_eq!(conn.get_object(&live).unwrap().as_ref(), b"live-payload");

    let orphan = conn.put_chunk(ObjectKind::Blob, b"orphan-payload").unwrap();
    let mut aborted = Batch::new();
    aborted.attach_object(&orphan, "pages/draft");
    aborted.abort();
    assert_eq!(conn.get("pages/draft").unwrap(), None);
    assert!(
        conn.scan_orphans("pages/")
            .unwrap()
            .iter()
            .any(|object| object.hash_hex() == orphan.hash_hex()),
        "gate G-YYDB-6 fixture yydb.object.batch_attach expected aborted attach to stay orphan"
    );
    cleanup(&path);
}
