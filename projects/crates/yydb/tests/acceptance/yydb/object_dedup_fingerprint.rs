// fixture: yydb.object.dedup_fingerprint

#[path = "../../fixtures/yydb/mod.rs"]
mod harness;
use harness::{cleanup, open_temp_db};
use yydb::ObjectKind;

#[test]
fn yydb_object_dedup_fingerprint() {
    let (conn, path) = open_temp_db("object-dedup");
    let payload = b"same-bytes-same-hash";
    let first = conn.put_chunk(ObjectKind::Blob, payload).unwrap();
    let second = conn.put_chunk(ObjectKind::Blob, payload).unwrap();
    assert_eq!(
        first.hash_hex(),
        second.hash_hex(),
        "fixture yydb.object.dedup_fingerprint hash mismatch"
    );
    cleanup(&path);
}
