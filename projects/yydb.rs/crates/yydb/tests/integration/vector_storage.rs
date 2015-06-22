//! `vector<N>` through the VOS author surface: declared dimension, row codec,
//! transaction boundary and reopen.
//!
//! A `vector<N>` column stores a fixed number of finite `f32` components. The
//! column contract owns the dimension, so a value of another shape is refused
//! before it reaches the row, and the stored form keeps the exact component
//! bits so a read never re-rounds an embedding.

use std::{collections::BTreeMap, fs};

use crate::fixtures::yydb::{cleanup, open_temp_db, temp_db};
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
    conn.scan_prefix("__yydb/v0/row/Doc/", None, 64)
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
    let embeddings = [
        [f32::MIN, 0.0, f32::MAX],
        [-0.0, f32::from_bits(1), 1.0 / 3.0],
        [f32::EPSILON, -1.5, 16_777_216.0],
    ];
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
    let embeddings = [
        [f32::MIN, 0.0, f32::MAX],
        [-0.0, f32::from_bits(1), 1.0 / 3.0],
        [f32::EPSILON, -1.5, 16_777_216.0],
    ];
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
