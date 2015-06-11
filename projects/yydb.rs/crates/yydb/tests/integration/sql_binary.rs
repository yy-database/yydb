//! BLOB values through the SQL parameter path.
//!
//! A `bytes` column carries arbitrary octets. Binding a parameter must move
//! those octets without a UTF-8 or `String` detour, so a payload that no text
//! spelling can represent still stores, compares, and returns verbatim.

use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
};

use yydb::{Connection, INLINE_BYTES_MAX, Value};

const BINARY_SCHEMA: &str = r#"
table Payload {
    @@payload_id: i64,
    label: utf8,
    body: bytes,
}
"#;

fn open() -> Connection {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema(BINARY_SCHEMA).expect("schema");
    conn
}

fn insert(conn: &Connection, id: i64, body: Value) -> yydb::Result<usize> {
    conn.execute_sql_write(
        "INSERT INTO Payload (payload_id, label, body) VALUES (?, ?, ?)",
        &[Value::I64(id), Value::Text(format!("row-{id}")), body],
    )
}

fn bodies(conn: &Connection, sql: &str, values: &[Value]) -> Vec<Vec<u8>> {
    conn.query_sql_with_params(sql, values)
        .expect("blob query")
        .iter()
        .map(|row| match row.get("body") {
            Some(Value::Bytes(bytes)) => bytes.clone(),
            other => panic!("unexpected body value {other:?}"),
        })
        .collect()
}

fn ids(conn: &Connection, sql: &str, values: &[Value]) -> Vec<i64> {
    conn.query_sql_with_params(sql, values)
        .expect("blob query")
        .iter()
        .map(|row| match row.get("payload_id") {
            Some(Value::I64(id)) => *id,
            other => panic!("unexpected payload_id {other:?}"),
        })
        .collect()
}

/// A payload that is neither valid UTF-8 nor free of NUL bytes.
fn non_text_octets(length: usize) -> Vec<u8> {
    (0..length).map(|index| if index % 7 == 0 { 0x00 } else { 0xff }).collect()
}

fn temp_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("yydb-sql-binary-{}-{label}.yydb", std::process::id()))
}

#[test]
fn blob_parameter_binds_raw_octets_without_a_text_detour() {
    let conn = open();
    let payload = vec![0x00, 0xff, 0x10, 0x80];
    insert(&conn, 1, Value::Bytes(payload.clone())).expect("insert blob parameter");

    // The parameter path matches the exact octets, and the projection hands the
    // same bytes back instead of a decoded string.
    assert_eq!(ids(&conn, "SELECT payload_id FROM Payload WHERE body = ?", &[Value::Bytes(payload.clone())]), vec![1]);
    assert_eq!(bodies(&conn, "SELECT body FROM Payload WHERE payload_id = 1", &[]), vec![payload]);
    assert_eq!(
        ids(&conn, "SELECT payload_id FROM Payload WHERE LENGTH(body) = ?", &[Value::I64(4)]),
        vec![1],
        "LENGTH must report the byte count"
    );
}

#[test]
fn blob_parameter_keeps_empty_nul_and_non_utf8_payloads_distinct() {
    let conn = open();
    let empty: Vec<u8> = Vec::new();
    let nul = vec![0x00];
    let invalid = vec![0xff, 0xfe];
    insert(&conn, 1, Value::Bytes(empty.clone())).expect("insert empty blob");
    insert(&conn, 2, Value::Bytes(nul.clone())).expect("insert NUL blob");
    insert(&conn, 3, Value::Bytes(invalid.clone())).expect("insert non-utf8 blob");
    insert(&conn, 4, Value::Null).expect("insert NULL blob");

    // An empty BLOB is a value, not a NULL: it matches an empty parameter and
    // reports length zero.
    assert_eq!(ids(&conn, "SELECT payload_id FROM Payload WHERE body = ?", &[Value::Bytes(empty.clone())]), vec![1]);
    assert_eq!(ids(&conn, "SELECT payload_id FROM Payload WHERE body = ?", &[Value::Bytes(nul)]), vec![2]);
    assert_eq!(ids(&conn, "SELECT payload_id FROM Payload WHERE body = ?", &[Value::Bytes(invalid.clone())]), vec![3]);
    assert_eq!(bodies(&conn, "SELECT body FROM Payload WHERE body = ?", &[Value::Bytes(invalid)]), vec![vec![0xff, 0xfe]]);

    // `NULL` never equals a parameter, because SQL comparisons over `NULL` are
    // unknown, while `IS NULL` finds it.
    assert!(ids(&conn, "SELECT payload_id FROM Payload WHERE body = ?", &[Value::Null]).is_empty());
    assert_eq!(ids(&conn, "SELECT payload_id FROM Payload WHERE body IS NULL", &[]), vec![4]);

    let lengths = conn
        .query_sql("SELECT payload_id, LENGTH(body) AS body_len FROM Payload ORDER BY payload_id")
        .expect("lengths");
    let lengths: Vec<(Value, Value)> =
        lengths.iter().map(|row| (row["payload_id"].clone(), row["body_len"].clone())).collect();
    assert_eq!(
        lengths,
        vec![
            (Value::I64(1), Value::I64(0)),
            (Value::I64(2), Value::I64(1)),
            (Value::I64(3), Value::I64(2)),
            (Value::I64(4), Value::Null),
        ]
    );
}

#[test]
fn blob_parameter_round_trips_at_the_inline_threshold_edges() {
    let conn = open();
    let sizes = [INLINE_BYTES_MAX - 1, INLINE_BYTES_MAX, INLINE_BYTES_MAX + 1];
    for (offset, size) in sizes.iter().enumerate() {
        let id = offset as i64 + 1;
        insert(&conn, id, Value::Bytes(non_text_octets(*size))).expect("insert threshold blob");
    }

    for (offset, size) in sizes.iter().enumerate() {
        let id = offset as i64 + 1;
        let expected = non_text_octets(*size);
        assert_eq!(
            ids(&conn, "SELECT payload_id FROM Payload WHERE body = ?", &[Value::Bytes(expected.clone())]),
            vec![id],
            "parameter equality at length {size}"
        );
        assert_eq!(bodies(&conn, "SELECT body FROM Payload WHERE payload_id = ?", &[Value::I64(id)]), vec![expected]);
        assert_eq!(
            ids(&conn, "SELECT payload_id FROM Payload WHERE LENGTH(body) = ?", &[Value::I64(*size as i64)]),
            vec![id],
            "byte length at {size}"
        );
    }
}

#[test]
fn blob_parameter_ready_once_binds_different_payloads() {
    let conn = open();
    let stmt = conn.prepare_sql("SELECT payload_id FROM Payload WHERE body = ?").expect("prepare");
    insert(&conn, 1, Value::Bytes(b"first".to_vec())).expect("insert");
    insert(&conn, 2, Value::Bytes(b"second".to_vec())).expect("insert");

    let first = conn.query_prepared_ordered(&stmt, &[Value::Bytes(b"first".to_vec())]).expect("bind first");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].get("payload_id"), Some(&Value::I64(1)));

    // Binding a second payload reuses the plan and never degrades the octets.
    let second = conn.query_prepared_ordered(&stmt, &[Value::Bytes(vec![0x00, 0xff])]).expect("bind second");
    assert!(second.is_empty(), "a payload that was never stored must not match");
}

#[test]
fn blob_parameter_survives_close_and_reopen() {
    let path = temp_path("reopen");
    let _ = fs::remove_file(&path);
    // A stored row is one leaf-page cell, so a durable payload must fit a page.
    // Payloads at and above `INLINE_BYTES_MAX` are covered by the in-memory
    // threshold test; this case proves the octets survive the file codec, which
    // tags them as hex and would expose any text detour.
    let payload = non_text_octets(1024);

    {
        let conn = Connection::open(&path).expect("open file");
        conn.ensure_schema(BINARY_SCHEMA).expect("schema");
        insert(&conn, 1, Value::Bytes(payload.clone())).expect("insert durable blob");
        insert(&conn, 2, Value::Bytes(Vec::new())).expect("insert empty");
        insert(&conn, 3, Value::Null).expect("insert null");
    }

    let reopened = Connection::open(&path).expect("reopen");
    reopened.ensure_schema(BINARY_SCHEMA).expect("re-ensure schema");
    assert_eq!(bodies(&reopened, "SELECT body FROM Payload WHERE payload_id = ?", &[Value::I64(1)]), vec![payload.clone()]);
    assert_eq!(
        ids(&reopened, "SELECT payload_id FROM Payload WHERE body = ?", &[Value::Bytes(payload)]),
        vec![1],
        "the reopened octets must bind back to the same row"
    );
    assert_eq!(bodies(&reopened, "SELECT body FROM Payload WHERE payload_id = ?", &[Value::I64(2)]), vec![Vec::<u8>::new()]);
    assert_eq!(ids(&reopened, "SELECT payload_id FROM Payload WHERE payload_id = 3 AND body IS NULL", &[]), vec![3]);
    drop(reopened);
    let _ = fs::remove_file(&path);
}

#[test]
fn blob_parameter_and_literal_cast_paths_agree() {
    let conn = open();
    insert(&conn, 1, Value::Bytes(b"ab".to_vec())).expect("insert");
    insert(&conn, 2, Value::Bytes(b"abc".to_vec())).expect("insert");
    insert(&conn, 3, Value::Bytes(Vec::new())).expect("insert");

    // Equality: the parameter and the explicit literal cast select the same row.
    assert_eq!(
        ids(&conn, "SELECT payload_id FROM Payload WHERE body = ?", &[Value::Bytes(b"ab".to_vec())]),
        ids(&conn, "SELECT payload_id FROM Payload WHERE body = CAST('ab' AS BLOB)", &[])
    );
    // Ordering: a parameter comparison agrees with the literal cast comparison.
    assert_eq!(
        ids(&conn, "SELECT payload_id FROM Payload WHERE body > ?", &[Value::Bytes(b"ab".to_vec())]),
        ids(&conn, "SELECT payload_id FROM Payload WHERE body > CAST('ab' AS BLOB)", &[])
    );
    // Empty: an empty parameter agrees with the empty literal cast.
    assert_eq!(
        ids(&conn, "SELECT payload_id FROM Payload WHERE body = ?", &[Value::Bytes(Vec::new())]),
        ids(&conn, "SELECT payload_id FROM Payload WHERE body = CAST('' AS BLOB)", &[])
    );
    // `NULL` on both paths stays unknown, so neither selects a stored row.
    assert!(ids(&conn, "SELECT payload_id FROM Payload WHERE body = ?", &[Value::Null]).is_empty());
    assert!(ids(&conn, "SELECT payload_id FROM Payload WHERE body = CAST(NULL AS BLOB)", &[]).is_empty());
    // `CAST(NULL AS BLOB)` is a legal way to write a stored NULL.
    insert(&conn, 4, Value::Null).expect("insert null");
    conn.execute_sql_write("INSERT INTO Payload (payload_id, label, body) VALUES (5, 'row-5', CAST(NULL AS BLOB))", &[])
        .expect("insert null literal cast");
    assert_eq!(ids(&conn, "SELECT payload_id FROM Payload WHERE body IS NULL ORDER BY payload_id", &[]), vec![4, 5]);
}

#[test]
fn cast_over_a_parameter_is_not_a_comparison_value() {
    let conn = open();
    insert(&conn, 1, Value::Bytes(b"ab".to_vec())).expect("insert");

    // A comparison value is baked into the plan, so only a constant `CAST` folds
    // into one. The parameter form is refused by name instead of silently
    // comparing against a column that does not exist, and the plain parameter
    // form keeps working.
    let error = conn
        .query_sql_with_params("SELECT payload_id FROM Payload WHERE body = CAST(? AS BLOB)", &[Value::Bytes(b"ab".to_vec())])
        .expect_err("CAST over a parameter as a comparison value must reject");
    assert_eq!(error.code(), "yydb.schema.invalid");
    assert!(format!("{error}").contains("CAST over a parameter"), "unexpected: {error}");
    assert_eq!(ids(&conn, "SELECT payload_id FROM Payload WHERE body = ?", &[Value::Bytes(b"ab".to_vec())]), vec![1]);
}

#[test]
fn explicit_cast_keeps_the_parameter_octets() {
    let conn = open();
    insert(&conn, 1, Value::Bytes(Vec::new())).expect("insert anchor row");

    // `CAST(? AS BLOB)` over a BLOB parameter is the identity, so no text
    // spelling is involved.
    let rows = conn
        .query_sql_with_params(
            "SELECT CAST(? AS BLOB) AS body, LENGTH(CAST(? AS BLOB)) AS body_len FROM Payload WHERE payload_id = 1",
            &[Value::Bytes(vec![0x00, 0xff]), Value::Bytes(vec![0x00, 0xff])],
        )
        .expect("cast blob parameter");
    assert_eq!(rows[0].get("body"), Some(&Value::Bytes(vec![0x00, 0xff])));
    assert_eq!(rows[0].get("body_len"), Some(&Value::I64(2)));

    // `CAST(? AS BLOB)` over a text parameter is the one explicit text to BLOB
    // conversion, and it uses the UTF-8 octets of the text.
    let rows = conn
        .query_sql_with_params(
            "SELECT CAST(? AS BLOB) AS body, LENGTH(CAST(? AS BLOB)) AS body_len FROM Payload WHERE payload_id = 1",
            &[Value::Text("ab".into()), Value::Text("ab".into())],
        )
        .expect("cast text parameter to blob");
    assert_eq!(rows[0].get("body"), Some(&Value::Bytes(b"ab".to_vec())));
    assert_eq!(rows[0].get("body_len"), Some(&Value::I64(2)));
}

#[test]
fn text_parameter_into_a_bytes_column_rejects_without_writing() {
    let conn = open();
    let error = insert(&conn, 1, Value::Text("text".into())).expect_err("text parameter into bytes column must reject");
    assert_eq!(error.code(), "yydb.schema.invalid");
    let message = format!("{error}");
    assert!(message.contains("BLOB") && message.contains("CAST"), "unexpected: {message}");
    assert!(conn.query_sql("SELECT payload_id FROM Payload").expect("read").is_empty(), "no row may be published");
}

#[test]
fn blob_parameter_into_a_text_column_rejects_without_writing() {
    let conn = open();
    let error = conn
        .execute_sql_write(
            "INSERT INTO Payload (payload_id, label, body) VALUES (?, ?, ?)",
            &[Value::I64(1), Value::Bytes(b"label".to_vec()), Value::Bytes(vec![0x01])],
        )
        .expect_err("bytes parameter into text column must reject");
    assert_eq!(error.code(), "yydb.schema.invalid");
    let message = format!("{error}");
    assert!(message.contains("BLOB") && message.contains("bytes"), "unexpected: {message}");
    assert!(conn.query_sql("SELECT payload_id FROM Payload").expect("read").is_empty());
}

#[test]
fn cast_blob_to_text_rejects_instead_of_lossy_decoding() {
    let conn = open();
    insert(&conn, 1, Value::Bytes(vec![0xff])).expect("insert");

    // Text cannot express arbitrary octets, so the conversion fails instead of
    // substituting replacement characters.
    let error = conn
        .query_sql("SELECT CAST(body AS TEXT) AS body FROM Payload WHERE payload_id = 1")
        .expect_err("CAST a BLOB to TEXT must reject");
    assert_eq!(error.code(), "yydb.schema.invalid");
    let message = format!("{error}");
    assert!(message.contains("TEXT") && message.contains("not supported"), "unexpected: {message}");
}

#[test]
fn blob_rows_carry_no_text_copy_in_the_record_map() {
    let conn = open();
    let payload = vec![0x00, 0xff];
    insert(&conn, 1, Value::Bytes(payload.clone())).expect("insert");

    // The stored row keeps a tagged `bytes` object with the lowercase hex of the
    // original octets, which proves the write path never decoded them.
    let records: BTreeMap<String, Vec<u8>> = conn
        .scan_prefix("__yydb/v0/row/Payload/", None, 16)
        .expect("scan rows")
        .into_iter()
        .collect();
    assert_eq!(records.len(), 1, "one payload row must be stored");
    let raw = records.values().next().expect("payload row stored");
    let raw = std::str::from_utf8(raw).expect("row json is utf-8");
    assert!(raw.contains("\"$yydb\":\"bytes\""), "unexpected stored row: {raw}");
    assert!(raw.contains("00ff"), "unexpected stored row: {raw}");
}
