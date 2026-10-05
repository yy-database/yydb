//! Stateful [`MemorySession`] JSON envelopes and VOS roundtrips.

#[path = "../fixtures/mod.rs"]
mod harness;
use serde_json::{json, Value as JsonValue};
use harness::{assert_ok_true, parse_json, USER_SCHEMA};
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
fn memory_session_query_after_ensure_schema() {
    let session = MemorySession::new().expect("memory");
    assert_ok_true(&session.ensure_schema(USER_SCHEMA));
    let payload = session.query("User.collect()");
    let parsed = parse_json(&payload);
    assert_eq!(parsed.get("ok"), Some(&JsonValue::Bool(true)));
    assert_eq!(parsed.get("rows"), Some(&json!([])));
}

#[test]
fn memory_session_query_without_catalog_returns_empty_rows() {
    let session = MemorySession::new().expect("memory");
    let parsed = parse_json(&session.query("User.collect()"));
    assert_eq!(parsed.get("ok"), Some(&JsonValue::Bool(true)));
    assert_eq!(parsed.get("rows"), Some(&json!([])));
}
