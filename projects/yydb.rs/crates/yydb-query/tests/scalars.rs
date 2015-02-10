//! Exact temporal and decimal values through the VOS write and read paths.

use std::collections::BTreeMap;

use vos::types::ResolvedContract;
use yydb_query::{execute, execute_write, resolved_contract_from_document};
use yydb_types::{Date, Decimal, Error, Time, Timestamp, Value};

const SCALAR_SCHEMA: &str = r#"
table Event {
    @@event_id: i64,
    happened_on: date,
    at_time: time,
    created_at: datetime,
    amount: Decimal<38, 2>,
}
"#;

const VALID_INSERT: &str = r#"Event::insert({
    event_id: 7,
    happened_on: "2024-02-29",
    at_time: "12:34:56.123456789",
    created_at: "2024-02-29T12:34:56.123456Z",
    amount: "1234.50",
})"#;

fn contract() -> ResolvedContract {
    resolved_contract_from_document(SCALAR_SCHEMA).expect("scalar schema resolves")
}

fn insert(source: &str) -> (ResolvedContract, BTreeMap<String, Vec<u8>>) {
    let contract = contract();
    let mut records = BTreeMap::new();
    execute_write(source, &contract, None, &mut records).expect("insert succeeds");
    (contract, records)
}

#[test]
fn exact_scalars_roundtrip_through_row_storage() {
    let (contract, records) = insert(VALID_INSERT);
    let rows = execute("Event.all().collect()", Some(&contract), &records).expect("read rows");
    assert_eq!(rows.len(), 1);

    let row = &rows[0];
    assert_eq!(row.get("happened_on"), Some(&Value::Date(Date::parse_iso("2024-02-29").unwrap())));
    assert_eq!(row.get("at_time"), Some(&Value::Time(Time::parse_iso("12:34:56.123456789").unwrap())));
    assert_eq!(row.get("created_at"), Some(&Value::Timestamp(Timestamp::parse_iso("2024-02-29T12:34:56.123456Z").unwrap())));
    assert_eq!(row.get("amount"), Some(&Value::Decimal(Decimal::parse("1234.50", 38, 2).unwrap())));
}

#[test]
fn out_of_range_and_malformed_scalars_are_rejected_without_writing() {
    let cases = [
        ("2023-02-29", "12:00:00", "2024-01-01T00:00:00Z", "1.00"),
        ("0000-12-31", "12:00:00", "2024-01-01T00:00:00Z", "1.00"),
        ("2024-01-01", "24:00:00", "2024-01-01T00:00:00Z", "1.00"),
        ("2024-01-01", "12:00:00", "2024-01-01T00:00:00.1234567Z", "1.00"),
        ("2024-01-01", "12:00:00", "2024-01-01T00:00:00+08:00", "1.00"),
        ("2024-01-01", "12:00:00", "2024-01-01T00:00:00Z", "1.234"),
        ("2024-01-01", "12:00:00", "2024-01-01T00:00:00Z", "123456789012345678901234567890123456789.00"),
    ];
    let contract = contract();
    for (date, time, timestamp, amount) in cases {
        let mut records = BTreeMap::new();
        let source = format!(
            r#"Event::insert({{
                event_id: 7,
                happened_on: "{date}",
                at_time: "{time}",
                created_at: "{timestamp}",
                amount: "{amount}",
            }})"#
        );
        let error = execute_write(&source, &contract, None, &mut records).expect_err("invalid scalar rejects the write");
        assert!(matches!(error, Error::Scalar { .. }), "unexpected error for {date} {time} {timestamp} {amount}: {error}");
        assert!(records.is_empty(), "rejected write mutated the record map");
    }
}

#[test]
fn unsupported_canonical_scalars_are_rejected_before_storage() {
    let contract = resolved_contract_from_document(
        r#"
table Payload {
    @@payload_id: i64,
    body: utf16,
}
"#,
    )
    .expect("utf16 is a canonical scalar");
    let mut records = BTreeMap::new();
    let error = execute_write(r#"Payload::insert({ payload_id: 1, body: "text" })"#, &contract, None, &mut records)
        .expect_err("utf16 has no execution scalar");
    assert!(matches!(error, Error::Schema { .. }), "unexpected error: {error}");
    assert_eq!(error.code(), "yydb.schema.invalid");
    assert!(records.is_empty(), "rejected write mutated the record map");
}

#[test]
fn decimal_negative_zero_normalizes_and_never_truncates() {
    let (contract, records) = insert(
        r#"Event::insert({
            event_id: 1,
            happened_on: "2024-01-01",
            at_time: "00:00:00",
            created_at: "2024-01-01T00:00:00Z",
            amount: "-0.00",
        })"#,
    );
    let rows = execute("Event.all().collect()", Some(&contract), &records).expect("read rows");
    assert_eq!(rows[0].get("amount"), Some(&Value::Decimal(Decimal::parse("0", 38, 2).unwrap())));
}
