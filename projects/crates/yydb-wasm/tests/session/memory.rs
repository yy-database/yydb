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
