// gate: G-YYDB-6
// fixture: yydb.object.dedup_fingerprint

mod common;

use common::{cleanup, open_temp_db};
use yydb::ObjectKind;

#[test]
fn g_yydb_object_dedup_fingerprint() {
    let (conn, path) = open_temp_db("object-dedup");
    let payload = b"same-bytes-same-hash";
    let first = conn.put_chunk(ObjectKind::Blob, payload).unwrap();
    let second = conn.put_chunk(ObjectKind::Blob, payload).unwrap();
    assert_eq!(
        first.hash_hex(),
        second.hash_hex(),
        "gate G-YYDB-6 fixture yydb.object.dedup_fingerprint hash mismatch"
    );
    cleanup(&path);
}
