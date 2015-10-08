//! `vector<N>` storage behavior at the row-codec level.
//!
//! The host-coverable parts of this surface migrated to the binding suites:
//! component round-trip and reopen, dimension mismatch, non-finite or
//! malformed encodings, wrong-column rejection, the inline/object placement
//! switch between 120 and 121 dimensions and oversized-dimension schema
//! refusal all live in `@yydb/yydb`'s `engine-depth` / vector tests and in
//! `yydb.py`'s `test_yydb.py`. The tests below stay Rust-side because they
//! assert storage internals no binding exposes: the binary row record layout,
//! corruption injected through the internal row key, the embedded CAS records
//! a single-file `.yydb` uses for object-backed vectors, and the no-half-published
//! state after a refused schema.

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

fn stored_row_bytes_for_table(conn: &Connection, table: &str) -> Vec<Vec<u8>> {
    conn.scan_prefix(&format!("__yydb/v0/row/{table}/"), None, 64)
        .expect("scan rows")
        .into_iter()
        .map(|(_, bytes)| bytes)
        .collect()
}

/// Build one binary `Doc` row whose vector payload disagrees with its dimension.
///
/// Row record layout: codec version `u8`, column count `u16`, null bitmap
/// `ceil(n/8)`, one `length u16 + name` per column in ascending name order, then
/// one tagged value per non-null column.
fn corrupt_vector_row(dimension: u16, components: &[f32]) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(1);
    out.extend_from_slice(&2u16.to_le_bytes());
    out.push(0);
    out.extend_from_slice(&(9u16).to_le_bytes());
    out.extend_from_slice(b"embedding");
    out.extend_from_slice(&(2u16).to_le_bytes());
    out.extend_from_slice(b"id");
    out.push(0x0B);
    out.extend_from_slice(&dimension.to_le_bytes());
    out.extend_from_slice(&((components.len() * 4) as u32).to_le_bytes());
    for component in components {
        out.extend_from_slice(&component.to_le_bytes());
    }
    out.push(0x02);
    out.extend_from_slice(&1i64.to_le_bytes());
    out
}

#[test]
fn vos_vector_row_roundtrips_exact_component_bits() {
    let conn = open_memory();
    let embedding = [0.25, -2.5, 4.0];
    insert(&conn, 1, vector(&embedding)).expect("insert vector");

    let rows = conn.query_vos("Doc.all().collect()").expect("read rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("embedding"), Some(&vector(&embedding)), "the stored row must keep exact component bits");
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
    conn.put(&key, corrupt_vector_row(3, &[1.0])).expect("stage truncated row");
    let error = conn.query_vos("Doc.all().collect()").expect_err("truncated vector row must reject");
    assert!(matches!(error, Error::Scalar { kind: "vector", .. }), "unexpected error: {error}");

    // A payload that disagrees with its own dimension marker is refused the
    // same way, so a padded payload cannot pretend to be a wider embedding.
    conn.put(&key, corrupt_vector_row(4, &[1.0, 2.0])).expect("stage padded row");
    let error = conn.query_vos("Doc.all().collect()").expect_err("padded vector row must reject");
    assert!(matches!(error, Error::Scalar { kind: "vector", .. }), "unexpected error: {error}");
}

#[test]
fn vos_vector_single_file_yydb_embeds_large_vector() {
    // 大维度 vector 走 CAS；单文件 `.yydb` 把对象作为分块记录写进主文件，
    // 支持但不建议（会撑大单文件并拖慢快照与备份）。
    let (conn, path) = open_temp_db("vos-vector-yydb-large");
    conn.ensure_schema(LARGE_DIM_SCHEMA).expect("schema");

    let expected = large_vector(121);
    insert_large(&conn, 1, expected.clone()).expect("single-file .yydb must embed large vectors");
    let rows = stored_row_bytes_for_table(&conn, "BigDoc");
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].len() < 256,
        "large vector row must keep an object reference instead of the inline payload, got {} bytes",
        rows[0].len()
    );

    drop(conn);
    let reopened = Connection::open(&path).expect("reopen");
    let rows = reopened.query_vos("BigDoc.all().collect()").expect("read embedded vector");
    assert_eq!(rows[0].get("embedding"), Some(&expected), "reopen must resolve the embedded vector payload");
    drop(reopened);

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
