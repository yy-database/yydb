use database::execution::engine::{Batch, Connection, ObjectKind};

use super::TempDb;

// fixture: yydb.object.batch_attach
#[test]
fn yydb_object_batch_attach() {
    let dir = TempDb::new("object-batch-attach");
    let conn = Connection::open(dir.yydb()).expect("open yydb");

    let live = conn.put_chunk(ObjectKind::Blob, b"live-payload").expect("store live object");
    let mut batch = Batch::new();
    batch.attach_object(&live, "pages/live");
    batch.commit(&conn).expect("commit live attachment");

    let orphan = conn.put_chunk(ObjectKind::Blob, b"orphan-payload").expect("store orphan object");
    let mut dropped = Batch::new();
    dropped.attach_object(&orphan, "pages/draft");
    drop(dropped);

    assert_eq!(conn.get("pages/live").expect("get live key"), Some(format!("yydb:object:{}", live.hash_hex()).into_bytes()));
    assert_eq!(&conn.get_object(&live).expect("read live object")[..], b"live-payload");
    assert_eq!(conn.get("pages/draft").expect("get draft key"), None);
    let orphans = conn.scan_orphans("pages/").expect("scan orphans");
    assert!(orphans.iter().any(|object| object.hash_hex() == orphan.hash_hex()));
}

// fixture: yydb.object.dedup_fingerprint
#[test]
fn yydb_object_dedup_fingerprint() {
    let dir = TempDb::new("object-dedup-fingerprint");
    let conn = Connection::open(dir.yydb()).expect("open yydb");
    let first = conn.put_chunk(ObjectKind::Blob, b"identical-bytes").expect("first put");
    let second = conn.put_chunk(ObjectKind::Blob, b"identical-bytes").expect("second put");
    assert_eq!(first.hash_hex(), second.hash_hex(), "identical bytes must dedupe to one fingerprint");
    assert_eq!(&conn.get_object(&second).expect("read deduped object")[..], b"identical-bytes");
}
