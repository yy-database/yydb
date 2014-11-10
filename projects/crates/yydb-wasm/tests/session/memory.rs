//! Stateful [`MemorySession`] JSON envelopes.

use crate::fixtures::{assert_ok_true, parse_json};
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
