//! Text and binary result formats over BLOB output.
//!
//! A row still holds typed values when it leaves execution, so encoding is where
//! a host decides whether a `bytes` cell becomes text or raw octets. These tests
//! fix that boundary: the binary format carries the octets with their kind and
//! length, and the text format refuses them by name instead of choosing a
//! spelling of its own.

use std::collections::BTreeMap;

use vos::types::ResolvedContract;
use yydb_query::{ColumnMeta, ResultCell, ResultFormat, encode_result, execute_sql, resolved_contract_from_document};
use yydb_types::{INLINE_BYTES_MAX, Value};

const BINARY_SCHEMA: &str = r#"
table Asset {
    @@asset_id: i64,
    label: utf8,
    payload: bytes,
}
"#;

const TYPED_SCHEMA: &str = r#"
table Event {
    @@event_id: i64,
    happened_on: date,
    amount: Decimal<12, 2>,
}
"#;

fn binary_contract() -> ResolvedContract {
    resolved_contract_from_document(BINARY_SCHEMA).expect("asset schema resolves")
}

fn typed_contract() -> ResolvedContract {
    resolved_contract_from_document(TYPED_SCHEMA).expect("event schema resolves")
}

fn insert(records: &mut BTreeMap<String, Vec<u8>>, contract: &ResolvedContract, id: i64, label: Value, payload: Value) {
    execute_sql(
        "INSERT INTO Asset (asset_id, label, payload) VALUES (?, ?, ?)",
        &[Value::I64(id), label, payload],
        contract,
        &[],
        records,
    )
    .expect("insert asset");
}

fn read(records: &mut BTreeMap<String, Vec<u8>>, contract: &ResolvedContract, sql: &str) -> Vec<BTreeMap<String, Value>> {
    execute_sql(sql, &[], contract, &[], records).expect("read asset").rows
}

/// A payload that is neither valid UTF-8 nor free of NUL bytes.
fn non_text_octets(length: usize) -> Vec<u8> {
    (0..length).map(|index| if index % 7 == 0 { 0x00 } else { 0xff }).collect()
}

#[test]
fn binary_format_returns_blob_octets_with_kind_and_length_metadata() {
    let contract = binary_contract();
    let mut records = BTreeMap::new();
    // An empty payload, a NUL byte, invalid UTF-8, and both sides of the
    // `INLINE_BYTES_MAX` threshold: every one must survive encoding unchanged.
    let payloads = [
        Vec::new(),
        vec![0x00],
        vec![0xff, 0xfe],
        non_text_octets(INLINE_BYTES_MAX - 1),
        non_text_octets(INLINE_BYTES_MAX),
        non_text_octets(INLINE_BYTES_MAX + 1),
    ];
    for (offset, payload) in payloads.iter().enumerate() {
        insert(&mut records, &contract, offset as i64 + 1, Value::Text(format!("row-{offset}")), Value::Bytes(payload.clone()));
    }

    let rows = read(&mut records, &contract, "SELECT asset_id, label, payload FROM Asset ORDER BY asset_id");
    let encoded = encode_result(&rows, ResultFormat::Binary).expect("binary result");

    assert_eq!(encoded.format, ResultFormat::Binary);
    assert_eq!(
        encoded.columns,
        vec![
            ColumnMeta { name: "asset_id".into(), kind: "i64" },
            ColumnMeta { name: "label".into(), kind: "text" },
            ColumnMeta { name: "payload".into(), kind: "bytes" },
        ],
        "columns are sorted by name and carry the kind of the values they hold"
    );
    assert_eq!(encoded.rows.len(), payloads.len());
    for (offset, (row, payload)) in encoded.rows.iter().zip(&payloads).enumerate() {
        assert_eq!(row[0], ResultCell::Text((offset as i64 + 1).to_string()), "integer cells keep the text spelling");
        assert_eq!(row[2], ResultCell::Bytes(payload.clone()), "BLOB octets must arrive unchanged at length {}", payload.len());
        assert_eq!(row[2].kind(), "bytes");
        assert_eq!(row[2].byte_length(), Some(payload.len()), "length metadata at {}", payload.len());
    }
}

#[test]
fn binary_format_keeps_null_and_empty_blob_distinct() {
    let contract = binary_contract();
    let mut records = BTreeMap::new();
    insert(&mut records, &contract, 1, Value::Text("row-1".into()), Value::Null);
    insert(&mut records, &contract, 2, Value::Text("row-2".into()), Value::Bytes(Vec::new()));

    let rows = read(&mut records, &contract, "SELECT payload FROM Asset ORDER BY asset_id");
    let encoded = encode_result(&rows, ResultFormat::Binary).expect("binary result");

    // `NULL` has no payload at all, so it has no length, while an empty BLOB is a
    // value whose length is zero.
    assert_eq!(encoded.rows[0], vec![ResultCell::Null]);
    assert_eq!(encoded.rows[0][0].byte_length(), None);
    assert_eq!(encoded.rows[1], vec![ResultCell::Bytes(Vec::new())]);
    assert_eq!(encoded.rows[1][0].byte_length(), Some(0));
    assert_eq!(encoded.columns, vec![ColumnMeta { name: "payload".into(), kind: "bytes" }]);
}

#[test]
fn text_format_keeps_null_and_empty_text_distinct() {
    let contract = binary_contract();
    let mut records = BTreeMap::new();
    insert(&mut records, &contract, 1, Value::Null, Value::Bytes(vec![0x00]));
    insert(&mut records, &contract, 2, Value::Text(String::new()), Value::Bytes(vec![0x01]));
    insert(&mut records, &contract, 3, Value::Text("é".into()), Value::Bytes(vec![0x02]));

    let rows = read(&mut records, &contract, "SELECT label FROM Asset ORDER BY asset_id");
    let encoded = encode_result(&rows, ResultFormat::Text).expect("text result over a text column");

    assert_eq!(encoded.format, ResultFormat::Text);
    assert_eq!(encoded.rows[0], vec![ResultCell::Null]);
    assert_eq!(encoded.rows[0][0].byte_length(), None);
    assert_eq!(encoded.rows[1], vec![ResultCell::Text(String::new())]);
    assert_eq!(encoded.rows[1][0].byte_length(), Some(0));
    // The length is the UTF-8 byte length, which is what a wire prefix needs.
    assert_eq!(encoded.rows[2], vec![ResultCell::Text("é".into())]);
    assert_eq!(encoded.rows[2][0].byte_length(), Some(2));
    assert_eq!(encoded.columns, vec![ColumnMeta { name: "label".into(), kind: "text" }]);
}

#[test]
fn text_and_binary_formats_agree_on_every_cell_without_bytes() {
    let contract = binary_contract();
    let mut records = BTreeMap::new();
    insert(&mut records, &contract, 1, Value::Text("ab".into()), Value::Bytes(b"ab".to_vec()));
    insert(&mut records, &contract, 2, Value::Null, Value::Bytes(b"cd".to_vec()));

    let rows = read(&mut records, &contract, "SELECT asset_id, label FROM Asset ORDER BY asset_id");
    let text = encode_result(&rows, ResultFormat::Text).expect("text result");
    let binary = encode_result(&rows, ResultFormat::Binary).expect("binary result");

    // The formats differ only in how a `bytes` cell is carried, so a row stream
    // without one encodes identically.
    assert_eq!(text.columns, binary.columns);
    assert_eq!(text.rows, binary.rows);
}

#[test]
fn text_format_refuses_a_bytes_column_instead_of_spelling_it() {
    let contract = binary_contract();
    let mut records = BTreeMap::new();
    insert(&mut records, &contract, 1, Value::Text("row-1".into()), Value::Bytes(vec![0x00, 0xff]));

    let rows = read(&mut records, &contract, "SELECT asset_id, label, payload FROM Asset");
    let error = encode_result(&rows, ResultFormat::Text).expect_err("a text result cannot carry a BLOB");

    assert_eq!(error.code(), "yydb.result.format");
    assert_eq!(error.operation(), "query");
    assert_eq!(error.params(), Some(vec![("column", "payload".to_owned()), ("kind", "bytes".to_owned())]));
    let message = format!("{error}");
    assert!(message.contains("payload") && message.contains("bytes"), "unexpected: {message}");
}

#[test]
fn text_format_refuses_utf8_bytes_too() {
    let contract = binary_contract();
    let mut records = BTreeMap::new();
    // These octets happen to be valid UTF-8, but the column is still a BLOB, so a
    // text result must not hand back a string that could be read as text.
    insert(&mut records, &contract, 1, Value::Text("row-1".into()), Value::Bytes(b"ab".to_vec()));

    let rows = read(&mut records, &contract, "SELECT payload FROM Asset");
    let error = encode_result(&rows, ResultFormat::Text).expect_err("a BLOB is refused even when its octets are UTF-8");
    assert_eq!(error.code(), "yydb.result.format");
    assert_eq!(error.params(), Some(vec![("column", "payload".to_owned()), ("kind", "bytes".to_owned())]));
}

#[test]
fn both_formats_refuse_a_kind_without_a_text_spelling() {
    let contract = typed_contract();
    let mut records = BTreeMap::new();
    execute_sql(
        "INSERT INTO Event (event_id, happened_on, amount) VALUES (1, '2024-02-29', '1.50')",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("insert event");

    let rows = execute_sql("SELECT amount FROM Event", &[], &contract, &[], &mut records).expect("read amount").rows;
    // A decimal has no text spelling on either surface (`CAST(… AS TEXT)` refuses
    // it too), so encoding names the kind instead of formatting it locally.
    for format in [ResultFormat::Text, ResultFormat::Binary] {
        let error = encode_result(&rows, format).expect_err("a decimal has no text spelling");
        assert_eq!(error.code(), "yydb.result.format");
        assert_eq!(error.params(), Some(vec![("column", "amount".to_owned()), ("kind", "decimal".to_owned())]));
    }

    let rows = execute_sql("SELECT happened_on FROM Event", &[], &contract, &[], &mut records).expect("read date").rows;
    let error = encode_result(&rows, ResultFormat::Binary).expect_err("a date has no text spelling");
    assert_eq!(error.params(), Some(vec![("column", "happened_on".to_owned()), ("kind", "date".to_owned())]));
}

#[test]
fn empty_row_stream_encodes_with_no_columns() {
    let contract = binary_contract();
    let mut records = BTreeMap::new();

    let rows = read(&mut records, &contract, "SELECT payload FROM Asset");
    let encoded = encode_result(&rows, ResultFormat::Binary).expect("empty binary result");
    assert!(encoded.rows.is_empty());
    assert!(encoded.columns.is_empty());
}
