//! Shared helpers for `yydb-wasm` integration and OPFS acceptance fixtures.

use serde_json::Value as JsonValue;

use yydb_wasm::OpfsCapabilities;

pub const USER_SCHEMA: &str = "table User { @@id: i64, name: utf8 }";

pub fn parse_json(payload: &str) -> JsonValue {
    serde_json::from_str(payload).expect("valid JSON envelope")
}

pub fn assert_ok_true(payload: &str) {
    let parsed = parse_json(payload);
    assert_eq!(parsed.get("ok"), Some(&JsonValue::Bool(true)));
}

pub fn assert_ok_false(payload: &str) {
    let parsed = parse_json(payload);
    assert_eq!(parsed.get("ok"), Some(&JsonValue::Bool(false)));
    assert!(
        parsed.get("error").and_then(JsonValue::as_str).is_some(),
        "expected error string in {payload}"
    );
}

pub fn full_yydb_caps() -> OpfsCapabilities {
    OpfsCapabilities::full_yydb_profile()
}

pub fn full_yydx_caps() -> OpfsCapabilities {
    OpfsCapabilities::full_yydx_profile()
}
