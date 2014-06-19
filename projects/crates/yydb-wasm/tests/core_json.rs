use serde_json::{json, Value as JsonValue};
use yydb_wasm::MemorySession;

#[test]
fn kv_get_json_returns_bytes() {
    let session = MemorySession::new().expect("memory");
    session.put("theme", b"dark");
    let payload = session.get("theme");
    let parsed: JsonValue = serde_json::from_str(&payload).expect("json");
    assert_eq!(parsed.get("ok"), Some(&JsonValue::Bool(true)));
    assert_eq!(parsed.get("value"), Some(&json!([100, 97, 114, 107])));
}

#[test]
fn schema_get_json_empty() {
    let session = MemorySession::new().expect("memory");
    let payload = session.get_schema();
    let parsed: JsonValue = serde_json::from_str(&payload).expect("json");
    assert_eq!(parsed.get("ok"), Some(&JsonValue::Bool(true)));
    assert!(parsed.get("schema").unwrap().is_null());
}
