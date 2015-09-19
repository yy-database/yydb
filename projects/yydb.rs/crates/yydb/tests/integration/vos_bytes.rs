//! `bytes` through the VOS author surface: object-store failure paths.
//!
//! The host-coverable parts of this surface migrated to the binding suites:
//! arbitrary-octet round-trip, the inline/object boundary, single-file
//! embedding, deduplication, rollback visibility and reopen are covered by
//! `@yydb/yydb`'s `bytes-channel` and `yydb.py`'s `test_yydb.py`. The test
//! below stays Rust-side because injecting a missing object requires direct
//! access to the internal object store.

use std::fs;

use crate::fixtures::yydb::{cleanup, open_temp_yydx};
use yydb::{Connection, Error, INLINE_BYTES_MAX, Value};

const BLOB_SCHEMA: &str = r#"
table Blob {
    @@blob_id: i64,
    label: utf8,
    body: bytes,
}
"#;

const INSERT: &str = r#"Blob::insert({ blob_id: $id, label: $label, body: $body })"#;

fn insert(conn: &Connection, id: i64, body: Value) -> yydb::Result<()> {
    conn.execute_vos_write_with_params(
        INSERT,
        &std::collections::BTreeMap::from([
            ("id".to_string(), Value::I64(id)),
            ("label".to_string(), Value::Text(format!("row-{id}"))),
            ("body".to_string(), body),
        ]),
    )
}

/// Octets that are neither valid UTF-8 nor free of NUL.
fn non_text_octets(length: usize) -> Vec<u8> {
    (0..length).map(|index| if index % 5 == 0 { 0x00 } else { 0xff }).collect()
}

#[test]
fn vos_bytes_missing_object_is_refused_at_read() {
    let (conn, path) = open_temp_yydx("vos-bytes-missing");
    conn.ensure_schema(BLOB_SCHEMA).expect("schema");
    insert(&conn, 1, Value::Bytes(non_text_octets(INLINE_BYTES_MAX + 1))).expect("insert object-backed blob");

    let objects = conn.objects().list_objects().expect("list objects");
    assert_eq!(objects.len(), 1);
    let object_path = conn.objects().path_for(objects.first().expect("one object"));
    assert!(object_path.exists(), "object must be on disk");
    drop(conn);

    // Losing the object leaves a row that cannot be decoded. The read fails
    // instead of substituting empty or text octets.
    fs::remove_file(&object_path).expect("remove object");
    let reopened = Connection::open_yydx(&path).expect("reopen");
    reopened.ensure_schema(BLOB_SCHEMA).expect("schema");
    let error = reopened.query_vos("Blob.all().collect()").expect_err("missing object must reject");
    assert!(matches!(error, Error::ObjectNotFound { .. }), "unexpected error: {error}");
    drop(reopened);
    cleanup(&path);
}
