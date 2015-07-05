//! `vector<N>` through the VOS author surface: declared dimension, row codec,
//! transaction boundary and reopen.
//!
//! A `vector<N>` column stores a fixed number of finite `f32` components. The
//! column contract owns the dimension, so a value of another shape is refused
//! before it reaches the row, and the stored form keeps the exact component
//! bits so a read never re-rounds an embedding.

use std::{collections::BTreeMap, fs};

use crate::fixtures::yydb::{cleanup, open_temp_db, open_temp_yydx, temp_db, temp_yydx};
use yydb::{Connection, Error, Value, Vector};

const DOC_SCHEMA: &str = r#"
table Doc {
    @@id: i64,
    embedding: vector<3>,
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

fn stored_components(conn: &Connection, id: i64) -> Vec<u32> {
    let rows = conn.query_vos("Doc.all().collect()").expect("read stored docs");
    let row = rows
        .into_iter()
        .find(|row| matches!(row.get("id"), Some(Value::I64(found)) if *found == id))
        .unwrap_or_else(|| panic!("row {id} not stored"));
    match row.get("embedding") {
        Some(Value::Vector(vector)) => vector.data.iter().map(|component| component.to_bits()).collect(),
        other => panic!("row {id} embedding must decode to a vector, got {other:?}"),
    }
}

fn bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|component| component.to_bits()).collect()
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

fn open_memory() -> Connection {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema(DOC_SCHEMA).expect("schema");
    conn
}

#[test]
fn vos_vector_roundtrips_component_bits_and_dimension() {
    let conn = open_memory();
    let embeddings = [[f32::MIN, 0.0, f32::MAX], [-0.0, f32::from_bits(1), 1.0 / 3.0], [f32::EPSILON, -1.5, 16_777_216.0]];
    for (offset, embedding) in embeddings.iter().enumerate() {
        insert(&conn, offset as i64 + 1, vector(embedding)).expect("insert vector");
    }

    for (offset, embedding) in embeddings.iter().enumerate() {
        assert_eq!(
            stored_components(&conn, offset as i64 + 1),
            bits(embedding),
            "component bits of embedding {offset} must survive the row codec"
        );
    }
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
fn vos_vector_dimension_mismatch_is_refused_before_publish() {
    let conn = open_memory();
    let error = insert(&conn, 1, vector(&[0.5, 1.5])).expect_err("a two-component vector cannot fill `vector<3>`");
    assert!(matches!(error, Error::Schema { .. }), "unexpected error: {error}");
    assert!(format!("{error}").contains("vector<3>"), "unexpected: {error}");
    assert!(stored_row_json(&conn).is_empty(), "no row may be published");
}

#[test]
fn vos_vector_empty_and_non_finite_values_are_refused_before_publish() {
    let conn = open_memory();
    let empty = insert(&conn, 1, vector(&[])).expect_err("an empty vector is not a `vector<3>`");
    assert!(matches!(empty, Error::Schema { .. }), "unexpected error: {empty}");

    let nan = insert(&conn, 2, vector(&[1.0, f32::NAN, 2.0])).expect_err("NaN is not a storable component");
    assert!(matches!(nan, Error::Schema { .. }), "unexpected error: {nan}");
    assert!(format!("{nan}").contains("finite"), "unexpected: {nan}");

    let infinite = insert(&conn, 3, vector(&[1.0, 2.0, f32::INFINITY])).expect_err("infinity is not a storable component");
    assert!(matches!(infinite, Error::Schema { .. }), "unexpected error: {infinite}");

    assert!(stored_row_json(&conn).is_empty(), "no row may be published");
}

#[test]
fn vos_vector_parameter_into_a_non_vector_column_rejects() {
    let conn = open_memory();
    let error = conn
        .execute_vos_write_with_params(
            r#"Doc::insert({ id: $id, embedding: $embedding })"#,
            &BTreeMap::from([
                ("id".to_string(), Value::Vector(Vector::new(vec![1.0, 2.0, 3.0]).expect("vector"))),
                ("embedding".to_string(), vector(&[1.0, 2.0, 3.0])),
            ]),
        )
        .expect_err("a vector parameter cannot fill the i64 primary key");
    assert!(matches!(error, Error::Schema { .. }), "unexpected error: {error}");
    assert!(stored_row_json(&conn).is_empty(), "no row may be published");

    let text = insert(&conn, 1, Value::Text("[1.0, 2.0, 3.0]".to_owned())).expect_err("text is not a vector");
    assert!(matches!(text, Error::Schema { .. }), "unexpected error: {text}");
    assert!(stored_row_json(&conn).is_empty(), "no row may be published");
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
fn vos_vector_rollback_leaves_no_visible_row() {
    let (conn, path) = open_temp_db("vos-vector-abort");
    conn.ensure_schema(DOC_SCHEMA).expect("schema");
    let embedding = [1.25, -3.5, 0.0];

    conn.begin().expect("begin");
    insert(&conn, 1, vector(&embedding)).expect("insert in txn");
    assert_eq!(stored_components(&conn, 1), bits(&embedding));
    conn.rollback().expect("rollback");

    assert!(stored_row_json(&conn).is_empty(), "aborted row must not be visible");
    drop(conn);

    let reopened = Connection::open(&path).expect("reopen");
    reopened.ensure_schema(DOC_SCHEMA).expect("schema");
    assert!(stored_row_json(&reopened).is_empty(), "reopened aborted row must stay invisible");
    drop(reopened);
    cleanup(&path);
}

#[test]
fn vos_vector_survives_close_and_reopen() {
    let path = temp_db("vos-vector-reopen");
    cleanup(&path);
    let embeddings = [[f32::MIN, 0.0, f32::MAX], [-0.0, f32::from_bits(1), 1.0 / 3.0], [f32::EPSILON, -1.5, 16_777_216.0]];
    {
        let conn = Connection::open(&path).expect("open");
        conn.ensure_schema(DOC_SCHEMA).expect("schema");
        for (offset, embedding) in embeddings.iter().enumerate() {
            insert(&conn, offset as i64 + 1, vector(embedding)).expect("insert durable vector");
        }
    }

    let reopened = Connection::open(&path).expect("reopen");
    reopened.ensure_schema(DOC_SCHEMA).expect("schema");
    for (offset, embedding) in embeddings.iter().enumerate() {
        assert_eq!(
            stored_components(&reopened, offset as i64 + 1),
            bits(embedding),
            "reopened component bits of embedding {offset}"
        );
    }
    drop(reopened);
    cleanup(&path);

    // The row files themselves are gone, so the next open starts empty.
    assert!(fs::metadata(&path).is_err(), "cleanup removed the database file");
}

// ---------------------------------------------------------------------------
// Large-dimension vector tests (方案 A: inline ≤ 120 维，> 120 维走 CAS 对象外置)
// ---------------------------------------------------------------------------

/// Schema with a large-dimension vector column that forces CAS placement.
const LARGE_DIM_SCHEMA: &str = r#"
table BigDoc {
    @@id: i64,
    embedding: vector<121>,
}
"#;

/// Schema right at the inline boundary (≤ 120 维应该 inline).
const BOUNDARY_SCHEMA: &str = r#"
table EdgeDoc {
    @@id: i64,
    embedding: vector<120>,
}
"#;

fn large_vector(dim: u32) -> Value {
    let data: Vec<f32> = (0..dim as usize).map(|i| i as f32 * 0.5).collect();
    Value::Vector(Vector::new(data).expect("vector"))
}

fn insert_large(conn: &Connection, id: i64, embedding: Value) -> yydb::Result<()> {
    conn.execute_vos_write_with_params(
        "BigDoc::insert({ id: $id, embedding: $embedding })",
        &BTreeMap::from([("id".to_string(), Value::I64(id)), ("embedding".to_string(), embedding)]),
    )
}

fn insert_edge(conn: &Connection, id: i64, embedding: Value) -> yydb::Result<()> {
    conn.execute_vos_write_with_params(
        "EdgeDoc::insert({ id: $id, embedding: $embedding })",
        &BTreeMap::from([("id".to_string(), Value::I64(id)), ("embedding".to_string(), embedding)]),
    )
}

#[test]
fn vos_vector_inline_boundary_n120_stays_inline_n121_uses_object() {
    // N=120 刚好在 inline 阈值内，row JSON 应该包含 tagged vector form。
    let path = temp_yydx("vos-vector-boundary");
    let conn = Connection::open_yydx(&path).expect("open_yydx");
    conn.ensure_schema(BOUNDARY_SCHEMA).expect("schema");
    insert_edge(&conn, 1, large_vector(120)).expect("insert 120-dim vector");

    let rows = stored_row_json_for_table(&conn, "EdgeDoc");
    assert_eq!(rows.len(), 1);
    assert!(rows[0].contains("\"$yydb\":\"vector\""), "N=120 must stay inline as tagged vector: {}", rows[0]);
    assert!(!rows[0].contains("\"$yydb\":\"object\""), "N=120 must NOT use object ref: {}", rows[0]);
    drop(conn);
    cleanup(&path);

    // N=121 超过 inline 阈值，row JSON 应该引用 CAS object。
    let path = temp_yydx("vos-vector-boundary-over");
    let conn = Connection::open_yydx(&path).expect("open_yydx");
    conn.ensure_schema(LARGE_DIM_SCHEMA).expect("schema");
    insert_large(&conn, 1, large_vector(121)).expect("insert 121-dim vector");

    let rows = stored_row_json_for_table(&conn, "BigDoc");
    assert_eq!(rows.len(), 1);
    assert!(rows[0].contains("\"$yydb\":\"object\""), "N=121 must use object ref: {}", rows[0]);
    assert!(rows[0].contains("vector-payload"), "N=121 object ref must carry kind=vector-payload: {}", rows[0]);
    drop(conn);
    cleanup(&path);
}

#[test]
fn vos_vector_large_dimension_roundtrips_via_yydx_object_store() {
    // 在 .yydx durable layout 下，大维度 vector 通过 CAS 外置，写入→读取→重开后组件必须一致。
    let path = temp_yydx("vos-vector-large-roundtrip");
    let conn = Connection::open_yydx(&path).expect("open_yydx");
    conn.ensure_schema(LARGE_DIM_SCHEMA).expect("schema");

    let expected: Vec<f32> = (0..121).map(|i| (i as f32) * 0.5 + 0.25).collect();
    insert_large(&conn, 1, Value::Vector(Vector::new(expected.clone()).expect("vector")))
        .expect("insert 121-dim vector");

    // 读回确认。
    let rows = conn.query_vos("BigDoc.all().collect()").expect("read");
    assert_eq!(rows.len(), 1);
    let stored = match rows[0].get("embedding") {
        Some(Value::Vector(v)) => v,
        other => panic!("expected Value::Vector, got {other:?}"),
    };
    assert_eq!(stored.data.len(), 121);
    for i in 0..121 {
        assert!((stored.data[i] - expected[i]).abs() < f32::EPSILON, "component {i} mismatch");
    }

    // 重开 .yydx，确认 persistent object 还在。
    drop(conn);
    let reopened = Connection::open_yydx(&path).expect("reopen");
    reopened.ensure_schema(LARGE_DIM_SCHEMA).expect("schema");
    let rows = reopened.query_vos("BigDoc.all().collect()").expect("read after reopen");
    assert_eq!(rows.len(), 1);
    let stored = match rows[0].get("embedding") {
        Some(Value::Vector(v)) => v,
        other => panic!("reopen: expected Value::Vector, got {other:?}"),
    };
    assert_eq!(stored.data.len(), 121);
    for i in 0..121 {
        assert!((stored.data[i] - expected[i]).abs() < f32::EPSILON, "reopen component {i} mismatch");
    }

    // 同时确认重开后 row JSON 里存的是 object ref（N=121 走 CAS）。
    let row_json = stored_row_json_for_table(&reopened, "BigDoc");
    assert_eq!(row_json.len(), 1);
    assert!(row_json[0].contains("\"$yydb\":\"object\""), "N=121 row must carry object ref: {}", row_json[0]);
    assert!(row_json[0].contains("vector-payload"), "object ref kind must be vector-payload: {}", row_json[0]);

    drop(reopened);
    cleanup(&path);
}

#[test]
fn vos_vector_single_file_yydb_rejects_large_vector() {
    // 大维度 vector 走 CAS，但 .yydb single-file mode 没有 persistent object store，
    // 所以写入应该被拒绝（和 bytes 的行为一致）。
    let (conn, path) = open_temp_db("vos-vector-yydb-reject-large");
    conn.ensure_schema(LARGE_DIM_SCHEMA).expect("schema");

    let error = insert_large(&conn, 1, large_vector(121)).expect_err("single-file .yydb must reject large vectors");
    assert!(matches!(error, Error::Unsupported(_)), "unexpected error: {error}");
    assert!(
        format!("{error}").contains(".yydx"),
        "single-file .yydb error should mention .yydx layout requirement: {error}"
    );
    assert!(stored_row_json_for_table(&conn, "BigDoc").is_empty(), "no row may be written");

    drop(conn);
    cleanup(&path);
}

#[test]
fn vos_vector_large_dimension_rollback_leaves_no_visible_row() {
    // rollback 后 row 消失（和 inline vector 行为一致）。
    // CAS object 的 orphan 清理是 reclaim_orphans 的职责，不在本测试覆盖范围。
    let (conn, path) = open_temp_yydx("vos-vector-large-abort");
    conn.ensure_schema(LARGE_DIM_SCHEMA).expect("schema");

    conn.begin().expect("begin");
    insert_large(&conn, 1, large_vector(121)).expect("insert in txn");

    // 在事务内，row 可读。
    let rows = conn.query_vos("BigDoc.all().collect()").expect("read in txn");
    assert_eq!(rows.len(), 1);
    drop(rows);

    conn.rollback().expect("rollback");

    // rollback 后 row 消失。
    assert!(
        stored_row_json_for_table(&conn, "BigDoc").is_empty(),
        "aborted row must not be visible"
    );

    drop(conn);
    cleanup(&path);
}

#[test]
fn vos_vector_max_dimension_large_object_payload() {
    // 768 维（常见 embedding 维度）远超过 inline 阈值，必须走 CAS object。
    let path = temp_yydx("vos-vector-768d");
    let conn = Connection::open_yydx(&path).expect("open");
    conn.ensure_schema(r#"
table Embeddings {
    @@id: i64,
    vec: vector<768>,
}
"#)
    .expect("schema");

    let data: Vec<f32> = (0..768).map(|i| (i as f32).sin()).collect();
    let bytes_len = data.len() * 4; // 768 * 4 = 3072 字节
    assert!(bytes_len > 1024, "768-dim payload (3072 bytes) must exceed INLINE_BYTES_MAX");

    conn.execute_vos_write_with_params(
        "Embeddings::insert({ id: $id, vec: $vec })",
        &BTreeMap::from([
            ("id".to_string(), Value::I64(1)),
            ("vec".to_string(), Value::Vector(Vector::new(data.clone()).expect("vector"))),
        ]),
    )
    .expect("insert 768-dim vector");

    // Row JSON 必须存 object ref，不是 inline vector。
    let rows = stored_row_json_for_table(&conn, "Embeddings");
    assert_eq!(rows.len(), 1);
    assert!(rows[0].contains("\"$yydb\":\"object\""), "768-dim must use object ref");
    assert!(rows[0].contains("vector-payload"), "kind must be vector-payload");

    // 读回验证。
    let result = conn.query_vos("Embeddings.all().collect()").expect("read");
    assert_eq!(result.len(), 1);
    let read_vec = match result[0].get("vec") {
        Some(Value::Vector(v)) => v,
        other => panic!("expected vector, got {other:?}"),
    };
    assert_eq!(read_vec.data.len(), 768);
    for i in 0..768 {
        assert!((read_vec.data[i] - data[i]).abs() < f32::EPSILON, "component {i} mismatch");
    }

    // 重开后继续一致。
    drop(conn);
    let reopened = Connection::open_yydx(&path).expect("reopen");
    reopened.ensure_schema(r#"
table Embeddings {
    @@id: i64,
    vec: vector<768>,
}
"#)
    .expect("schema");
    let result = reopened.query_vos("Embeddings.all().collect()").expect("read after reopen");
    assert_eq!(result.len(), 1);
    let read_vec = match result[0].get("vec") {
        Some(Value::Vector(v)) => v,
        other => panic!("reopen: expected vector, got {other:?}"),
    };
    assert_eq!(read_vec.data.len(), 768);
    for i in 0..768 {
        assert!((read_vec.data[i] - data[i]).abs() < f32::EPSILON, "reopen component {i} mismatch");
    }

    drop(reopened);
    cleanup(&path);
}
