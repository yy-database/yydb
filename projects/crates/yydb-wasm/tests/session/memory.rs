//! Stateful [`MemorySession`] JSON envelopes.

use crate::fixtures::{USER_SCHEMA, assert_ok_true, parse_json};
use serde_json::{Value as JsonValue, json};
use yydb_wasm::MemorySession;

#[test]
fn memory_session_kv_roundtrip() {
    let session = MemorySession::new().expect("memory");
    assert_ok_true(&session.put("theme", b"dark"));
    let payload = session.get("theme");
    let parsed = parse_json(&payload);
    assert_eq!(parsed.get("ok"), Some(&JsonValue::Bool(true)));
    assert_eq!(parsed.get("value"), Some(&json!([100, 97, 114, 107])));
}

const SCALAR_SCHEMA: &str = r#"
table Event {
    @@event_id: i64,
    @label: utf8,
    happened_on: date,
    at_time: time,
    created_at: datetime,
    amount: Decimal<38, 2>,
}
"#;

#[test]
fn memory_session_query_emits_tagged_exact_scalars() {
    let session = MemorySession::new().expect("memory");
    assert_ok_true(&session.ensure_schema(SCALAR_SCHEMA));
    assert_ok_true(&session.execute_vos_write(
        r#"Event::insert({
            event_id: 1,
            label: "ada",
            happened_on: "2024-02-29",
            at_time: "12:34:56.123456789",
            created_at: "2024-02-29T12:34:56.123456Z",
            amount: "1234.50",
        })"#,
    ));
    let parsed = parse_json(&session.query_vos("Event.all().collect()"));
    assert_eq!(parsed.get("ok"), Some(&JsonValue::Bool(true)));
    let rows = parsed.get("rows").and_then(JsonValue::as_array).expect("rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("happened_on"), Some(&json!({ "$yydb": "date", "days": 19782 })));
    assert_eq!(rows[0].get("at_time"), Some(&json!({ "$yydb": "time", "nanos": 45296123456789_i64 })));
    assert_eq!(rows[0].get("created_at"), Some(&json!({ "$yydb": "timestamp", "utcMicros": "1709210096123456" })));
    assert_eq!(
        rows[0].get("amount"),
        Some(&json!({ "$yydb": "decimal", "precision": 38, "scale": 2, "unscaled": "123450" }))
    );
}

#[test]
fn memory_session_reports_unique_violations_as_structured_errors() {
    let session = MemorySession::new().expect("memory");
    assert_ok_true(&session.ensure_schema(SCALAR_SCHEMA));
    let insert = |event_id: i64| {
        format!(
            r#"Event::insert({{
                event_id: {event_id},
                label: "ada",
                happened_on: "2024-02-29",
                at_time: "12:34:56",
                created_at: "2024-02-29T12:34:56Z",
                amount: "1.00",
            }})"#
        )
    };
    assert_ok_true(&session.execute_vos_write(&insert(1)));
    let parsed = parse_json(&session.execute_vos_write(&insert(2)));
    assert_eq!(parsed.get("ok"), Some(&JsonValue::Bool(false)));
    let code = parsed.get("error").and_then(|error| error.get("code")).and_then(JsonValue::as_str);
    assert_eq!(code, Some("yydb.unique.violation"));
    assert_eq!(parse_json(&session.query_vos("Event.all().collect()"))["rows"].as_array().unwrap().len(), 1);
}

#[test]
fn memory_session_schema_starts_empty() {
    let session = MemorySession::new().expect("memory");
    let payload = session.get_schema();
    let parsed = parse_json(&payload);
    assert_eq!(parsed.get("ok"), Some(&JsonValue::Bool(true)));
    assert!(parsed.get("schema").unwrap().is_null());
}

#[test]
fn memory_session_query_prepared_rejects_after_close() {
    let mut session = MemorySession::new().expect("memory");
    assert_ok_true(&session.ensure_schema(USER_SCHEMA));
    assert_ok_true(&session.execute_vos_write(r#"User.insert({ id: 1, name: "alice" })"#));

    let prepare = parse_json(&session.prepare_json("User.all().collect()"));
    assert_eq!(prepare.get("ok"), Some(&JsonValue::Bool(true)), "prepare must succeed");
    let stmt_id = prepare.get("statement_id").and_then(|v| v.as_str()).expect("statement_id present").to_owned();

    // Before close — execution succeeds.
    let before = parse_json(&session.query_prepared_json(&json!({ "statementId": stmt_id, "params": {} }).to_string()));
    assert_eq!(before.get("ok"), Some(&JsonValue::Bool(true)), "pre-close query must succeed");
    let rows = before.get("rows").and_then(|v| v.as_array()).expect("rows array");
    assert_eq!(rows.len(), 1);

    session.close();

    // After close — same handle is rejected with SessionClosed.
    let after = parse_json(&session.query_prepared_json(&json!({ "statementId": stmt_id, "params": {} }).to_string()));
    assert_eq!(after.get("ok"), Some(&JsonValue::Bool(false)), "post-close query must fail");
    let err = after.get("error").and_then(|v| v.as_str()).expect("error message present");
    assert!(err.contains("session closed"), "expected 'session closed' error, got: {err}");
}
