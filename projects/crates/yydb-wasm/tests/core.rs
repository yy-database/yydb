use serde_json::Value as JsonValue;
use yydb_wasm::{check_schema_source, query_memory};

#[test]
fn check_schema_source_counts_tables() {
    let check = check_schema_source("table User { @@id: i64, name: utf8 }");
    assert!(check.ok);
    assert_eq!(check.table_count, 1);
    assert!(!check.schema_fingerprint.is_empty());
}

#[test]
fn query_memory_returns_json_envelope() {
    let payload = query_memory("table User { @@id: i64, name: utf8 }");
    let parsed: JsonValue = serde_json::from_str(&payload).expect("json");
    assert_eq!(parsed.get("ok"), Some(&JsonValue::Bool(false)));
}
