//! `vector<N>` storage behavior at the row-codec level.
//!
//! The host-coverable parts of this surface migrated to the binding suites:
//! component round-trip and reopen, dimension mismatch, non-finite or
//! malformed encodings, wrong-column rejection, the inline/object placement
//! switch between 120 and 121 dimensions and oversized-dimension schema
//! refusal all live in `@yydb/yydb`'s `engine-depth` / vector tests and in
//! `yydb.py`'s `test_yydb.py`. The tests below stay Rust-side because they
//! assert storage internals no binding exposes: the raw row JSON layout,
//! corruption injected through the internal row key, the single-file layout
//! refusal for object-backed vectors, and the no-half-published state after
//! a refused schema.

use std::collections::BTreeMap;

use crate::fixtures::yydb::{cleanup, open_temp_db};
use yydb::{Connection, Error, Value, Vector};

const DOC_SCHEMA: &str = r#"
table Doc {
    @@id: i64,
    embedding: vector<3>,
}
"#;

const LARGE_DIM_SCHEMA: &str = r#"
table BigDoc {
    @@id: i64,
    embedding: vector<121>,
}
"#;

/// Schema with a vector column one dimension over the YYDB maximum.
const OVER_DIM_SCHEMA: &str = r#"
table OverMax {
    @@id: i64,
    embedding: vector<16385>,
}
"#;

const INSERT: &str = r#"Doc::insert({ id: $id, embedding: $embedding })"#;

fn vector(values: &[f32]) -> Value {
    Value::Vector(Vector::new(values.to_vec()).expect("vector"))
}

fn insert(conn: &Connection, id: i64, embedding: Value) -> yydb::Result<()> {
    conn.execute_vos_write_with_params(
        INSERT,
        &BTreeMap::from([("id".to_string(), Value::I64(id)), ("embedding".to_string(), embedding)]),
    )
}

fn insert_large(conn: &Connection, id: i64, embedding: Value) -> yydb::Result<()> {
    conn.execute_vos_write_with_params(
        "BigDoc::insert({ id: $id, embedding: $embedding })",
        &BTreeMap::from([("id".to_string(), Value::I64(id)), ("embedding".to_string(), embedding)]),
    )
}

fn large_vector(dim: u32) -> Value {
    let data: Vec<f32> = (0..dim as usize).map(|i| i as f32 * 0.5).collect();
    Value::Vector(Vector::new(data).expect("vector"))
}

fn open_memory() -> Connection {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema(DOC_SCHEMA).expect("schema");
    conn
}

fn stored_row_json(conn: &Connection) -> Vec<String> {
    stored_row_json_for_table(conn, "Doc")
}

fn stored_row_json_for_table(conn: &Connection, table: &str) -> Vec<String> {
    conn.scan_prefix(&format!("__yydb/v0/row/{table}/"), None, 64)
        .expect("scan rows")
        .into_iter()
        .map(|(_, bytes)| String::from_utf8(bytes).expect("row json is utf-8"))
        .collect()
}

#[test]
fn vos_vector_row_uses_the_tagged_vector_form() {
    let conn = open_memory();
    let embedding = [0.25, -2.5, 4.0];
    insert(&conn, 1, vector(&embedding)).expect("insert vector");

    let rows = stored_row_json(&conn);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert!(row.contains("\"$yydb\":\"vector\""), "row must carry the tagged vector form: {row}");
    assert!(row.contains("\"dimension\":3"), "row must record the declared dimension: {row}");
    let expected = Vector::new(embedding.to_vec()).expect("vector").to_le_bytes();
    let hex: String = expected.iter().map(|byte| format!("{byte:02x}")).collect();
    assert!(row.contains(&hex), "row must carry the exact component bits `{hex}`: {row}");
}

#[test]
fn vos_vector_corrupt_row_encoding_is_refused_at_read() {
    let conn = open_memory();
    insert(&conn, 1, vector(&[1.0, 2.0, 3.0])).expect("insert vector");
    let key = conn
        .scan_prefix("__yydb/v0/row/Doc/", None, 64)
        .expect("scan rows")
        .into_iter()
        .map(|(key, _)| key)
        .next()
        .expect("one stored row");

    // A row whose component payload is shorter than the declared dimension
    // cannot be decoded as an embedding, so the read fails instead of
    // substituting a shorter vector.
    let truncated = br#"{"id":1,"embedding":{"$yydb":"vector","dimension":3,"components":"0000803f"}}"#;
    conn.put(&key, truncated).expect("stage truncated row");
    let error = conn.query_vos("Doc.all().collect()").expect_err("truncated vector row must reject");
    assert!(matches!(error, Error::Scalar { kind: "vector", .. }), "unexpected error: {error}");

    // A payload that disagrees with its own dimension marker is refused the
    // same way, so a padded payload cannot pretend to be a wider embedding.
    let padded = br#"{"id":1,"embedding":{"$yydb":"vector","dimension":4,"components":"0000803f0000803f"}}"#;
    conn.put(&key, padded).expect("stage padded row");
    let error = conn.query_vos("Doc.all().collect()").expect_err("padded vector row must reject");
    assert!(matches!(error, Error::Scalar { kind: "vector", .. }), "unexpected error: {error}");
}

#[test]
fn vos_vector_single_file_yydb_rejects_large_vector() {
    // 大维度 vector 走 CAS，但 .yydb single-file mode 没有 persistent object store，
    // 所以写入应该被拒绝（和 bytes 的行为一致）。
    let (conn, path) = open_temp_db("vos-vector-yydb-reject-large");
    conn.ensure_schema(LARGE_DIM_SCHEMA).expect("schema");

    let error = insert_large(&conn, 1, large_vector(121)).expect_err("single-file .yydb must reject large vectors");
    assert!(matches!(error, Error::Unsupported(_)), "unexpected error: {error}");
    assert!(format!("{error}").contains(".yydx"), "single-file .yydb error should mention .yydx layout requirement: {error}");
    assert!(stored_row_json_for_table(&conn, "BigDoc").is_empty(), "no row may be written");

    drop(conn);
    cleanup(&path);
}

#[test]
fn vos_vector_excessive_dimension_error_persists_across_reopen() {
    // 超界 schema 根本无法落库，重开后也不会出现半发布状态。
    let (conn, path) = open_temp_db("vos-vector-overmax");
    let error = conn.ensure_schema(OVER_DIM_SCHEMA).expect_err("must reject at ensure_schema");
    assert!(matches!(error, Error::VectorDimensionExceeded { .. }), "error kind must be VectorDimensionExceeded, got: {error}");
    drop(conn);

    // 重开数据库，不应该有残留的 schema 状态。
    let reopened = Connection::open(&path).expect("reopen");
    // 正常 schema（vector<3>）应该可以成功。
    reopened.ensure_schema(DOC_SCHEMA).expect("after reject, normal schema must still work");
    drop(reopened);
    cleanup(&path);
}
