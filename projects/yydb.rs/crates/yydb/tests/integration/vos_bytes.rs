//! `bytes` through the VOS author surface: field, parameter binding, row codec,
//! transaction boundary and reopen.
//!
//! A `bytes` column must move octets verbatim. Values above the storage's own
//! inline boundary become a content-addressed object reference in the row, so
//! these cases also pin the boundary (`INLINE_BYTES_MAX - 1 / N / N + 1`),
//! deduplication, missing-object rejection and rollback visibility.

use std::{collections::BTreeMap, fs};

use yydb::{Connection, Error, INLINE_BYTES_MAX, Value};
use crate::fixtures::yydb::{cleanup, open_temp_yydx, temp_yydx};

const BLOB_SCHEMA: &str = r#"
table Blob {
    @@blob_id: i64,
    label: utf8,
    body: bytes,
}
"#;

const INSERT: &str = r#"Blob::insert({ blob_id: $id, label: $label, body: $body })"#;

fn params(id: i64, body: Value) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("id".to_string(), Value::I64(id)),
        ("label".to_string(), Value::Text(format!("row-{id}"))),
        ("body".to_string(), body),
    ])
}

fn insert(conn: &Connection, id: i64, body: Value) -> yydb::Result<()> {
    conn.execute_vos_write_with_params(INSERT, &params(id, body))
}

/// Octets that are neither valid UTF-8 nor free of NUL.
fn non_text_octets(length: usize) -> Vec<u8> {
    (0..length).map(|index| if index % 5 == 0 { 0x00 } else { 0xff }).collect()
}

fn stored_bodies(conn: &Connection) -> BTreeMap<i64, Value> {
    conn.query_vos("Blob.all().collect()")
        .expect("read stored blobs")
        .into_iter()
        .map(|row| {
            let id = match row.get("blob_id") {
                Some(Value::I64(id)) => *id,
                other => panic!("unexpected blob_id {other:?}"),
            };
            (id, row.get("body").expect("body projected").clone())
        })
        .collect()
}

fn stored_body(conn: &Connection, id: i64) -> Value {
    stored_bodies(conn).remove(&id).unwrap_or_else(|| panic!("row {id} not stored"))
}

fn stored_body_octets(conn: &Connection, id: i64) -> Vec<u8> {
    match stored_body(conn, id) {
        Value::Bytes(octets) => octets,
        other => panic!("row {id} body must decode to bytes, got {other:?}"),
    }
}

fn stored_row_json(conn: &Connection) -> Vec<String> {
    conn.scan_prefix("__yydb/v0/row/Blob/", None, 64)
        .expect("scan rows")
        .into_iter()
        .map(|(_, bytes)| String::from_utf8(bytes).expect("row json is utf-8"))
        .collect()
}

fn open_memory() -> Connection {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema(BLOB_SCHEMA).expect("schema");
    conn
}

#[test]
fn vos_bytes_parameter_carries_arbitrary_octets_verbatim() {
    let conn = open_memory();
    let empty: Vec<u8> = Vec::new();
    let nul = vec![0x00];
    let invalid_utf8 = vec![0xff, 0xfe, 0x80];
    let mixed = non_text_octets(64);

    insert(&conn, 1, Value::Bytes(empty.clone())).expect("insert empty");
    insert(&conn, 2, Value::Bytes(nul.clone())).expect("insert NUL");
    insert(&conn, 3, Value::Bytes(invalid_utf8.clone())).expect("insert invalid utf-8");
    insert(&conn, 4, Value::Bytes(mixed.clone())).expect("insert mixed");
    insert(&conn, 5, Value::Null).expect("insert NULL");

    // Every payload round-trips as bytes, never as text or a replacement string.
    assert_eq!(stored_body_octets(&conn, 1), empty);
    assert_eq!(stored_body_octets(&conn, 2), nul);
    assert_eq!(stored_body_octets(&conn, 3), invalid_utf8);
    assert_eq!(stored_body_octets(&conn, 4), mixed);
    assert_eq!(stored_body(&conn, 5), Value::Null);

    // An empty payload is a value, not a NULL.
    assert_ne!(stored_body(&conn, 1), Value::Null);
}

#[test]
fn vos_bytes_boundary_places_inline_at_and_below_and_object_above() {
    let conn = open_memory();
    let sizes = [INLINE_BYTES_MAX - 1, INLINE_BYTES_MAX, INLINE_BYTES_MAX + 1];
    for (offset, size) in sizes.iter().enumerate() {
        let id = offset as i64 + 1;
        insert(&conn, id, Value::Bytes(non_text_octets(*size))).expect("insert threshold blob");
    }

    for (offset, size) in sizes.iter().enumerate() {
        let id = offset as i64 + 1;
        assert_eq!(stored_body_octets(&conn, id), non_text_octets(*size), "round trip at length {size}");
    }

    // The row codec records the placement: inline payloads stay `$yydb: bytes`,
    // the payload above the boundary becomes an object reference.
    let rows = stored_row_json(&conn);
    assert_eq!(rows.len(), sizes.len());
    let inline = rows.iter().filter(|row| row.contains("\"$yydb\":\"bytes\"")).count();
    let objects = rows.iter().filter(|row| row.contains("\"$yydb\":\"object\"")).count();
    assert_eq!(inline, 2, "rows at N - 1 and N must stay inline: {rows:?}");
    assert_eq!(objects, 1, "row at N + 1 must reference an object: {rows:?}");

    assert_eq!(conn.objects().list_objects().expect("list objects").len(), 1);
}

#[test]
fn vos_bytes_above_the_boundary_need_a_persistent_object_layout() {
    let path = temp_yydx("vos-bytes-single-file");
    cleanup(&path);
    let single_file = path.with_extension("yydb");

    let conn = Connection::open(&single_file).expect("open .yydb");
    conn.ensure_schema(BLOB_SCHEMA).expect("schema");
    insert(&conn, 1, Value::Bytes(non_text_octets(64))).expect("inline payload fits a .yydb");

    // A single-file `.yydb` has no durable object root, so a payload above the
    // boundary would lose its octets on reopen. The write is refused instead.
    let error = insert(&conn, 2, Value::Bytes(non_text_octets(INLINE_BYTES_MAX + 1)))
        .expect_err("object payload in single-file mode must reject");
    assert!(matches!(error, Error::Unsupported(_)), "unexpected error: {error}");
    assert!(format!("{error}").contains(".yydx"), "unexpected: {error}");
    assert_eq!(stored_bodies(&conn).len(), 1, "no second row may be published");

    drop(conn);
    cleanup(&single_file);
    cleanup(&path);
}

#[test]
fn vos_bytes_identical_payloads_share_one_object() {
    let conn = open_memory();
    let payload = non_text_octets(INLINE_BYTES_MAX + 512);
    insert(&conn, 1, Value::Bytes(payload.clone())).expect("insert first");
    insert(&conn, 2, Value::Bytes(payload.clone())).expect("insert second");

    assert_eq!(stored_body_octets(&conn, 1), payload);
    assert_eq!(stored_body_octets(&conn, 2), payload);

    let objects = conn.objects().list_objects().expect("list objects");
    assert_eq!(objects.len(), 1, "identical octets must deduplicate to one object");
    let object = objects.first().expect("one object");
    assert_eq!(object.size, payload.len() as u64);
}

#[test]
fn vos_bytes_rollback_leaves_no_visible_row_or_reference() {
    let (conn, path) = open_temp_yydx("vos-bytes-abort");
    conn.ensure_schema(BLOB_SCHEMA).expect("schema");
    let payload = non_text_octets(INLINE_BYTES_MAX + 1);

    conn.begin().expect("begin");
    insert(&conn, 1, Value::Bytes(payload.clone())).expect("insert in txn");
    assert_eq!(stored_body_octets(&conn, 1), payload);
    conn.rollback().expect("rollback");

    assert!(stored_bodies(&conn).is_empty(), "aborted row must not be visible");
    assert!(stored_row_json(&conn).is_empty(), "aborted row reference must not be published");
    drop(conn);

    // The aborted reference must not survive the file either.
    let reopened = Connection::open_yydx(&path).expect("reopen");
    reopened.ensure_schema(BLOB_SCHEMA).expect("schema");
    assert!(stored_bodies(&reopened).is_empty(), "reopened aborted row must stay invisible");

    // A committed write of the same payload is visible after another reopen.
    insert(&reopened, 1, Value::Bytes(payload.clone())).expect("commit after rollback");
    drop(reopened);
    let reopened = Connection::open_yydx(&path).expect("reopen after commit");
    reopened.ensure_schema(BLOB_SCHEMA).expect("schema");
    assert_eq!(stored_body_octets(&reopened, 1), payload);
    drop(reopened);
    cleanup(&path);
}

#[test]
fn vos_bytes_survive_close_and_reopen_with_objects() {
    let (conn, path) = open_temp_yydx("vos-bytes-reopen");
    conn.ensure_schema(BLOB_SCHEMA).expect("schema");
    // A durable inline row is one leaf-page cell, so the at-or-below-boundary
    // case uses a page-sized payload. Payloads above the boundary stay small in
    // the row because only the reference is stored.
    let payloads = [Vec::new(), non_text_octets(64), non_text_octets(INLINE_BYTES_MAX + 1)];
    for (offset, payload) in payloads.iter().enumerate() {
        insert(&conn, offset as i64 + 1, Value::Bytes(payload.clone())).expect("insert durable blob");
    }
    drop(conn);

    let reopened = Connection::open_yydx(&path).expect("reopen");
    reopened.ensure_schema(BLOB_SCHEMA).expect("schema");
    for (offset, payload) in payloads.iter().enumerate() {
        assert_eq!(stored_body_octets(&reopened, offset as i64 + 1), *payload, "reopened octets at index {offset}");
    }
    drop(reopened);
    cleanup(&path);
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

#[test]
fn vos_bytes_parameter_into_a_text_column_rejects_without_writing() {
    let conn = open_memory();
    let error = conn
        .execute_vos_write_with_params(
            r#"Blob::insert({ blob_id: $id, label: $label, body: $body })"#,
            &BTreeMap::from([
                ("id".to_string(), Value::I64(1)),
                ("label".to_string(), Value::Bytes(b"label".to_vec())),
                ("body".to_string(), Value::Bytes(vec![0x01])),
            ]),
        )
        .expect_err("bytes parameter into a utf8 column must reject");
    assert!(matches!(error, Error::Schema { .. }), "unexpected error: {error}");
    assert!(stored_bodies(&conn).is_empty(), "no row may be published");
}

#[test]
fn vos_bytes_write_refuses_a_read_pipeline() {
    let conn = open_memory();
    let error = conn
        .execute_vos_write_with_params("Blob.all().collect()", &BTreeMap::new())
        .expect_err("a read pipeline must not enter the DML executor");
    assert!(matches!(error, Error::Unsupported(_)), "unexpected error: {error}");
}
