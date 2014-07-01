//! Stateless schema validation and introspection (no build version in payloads).

use serde_json::Value as JsonValue;
use crate::fixtures::{parse_json, USER_SCHEMA};
use yydb_wasm::{check_schema_source, introspect_schema_json};
#[test]
fn check_schema_source_counts_tables() {
    let check = check_schema_source(USER_SCHEMA);
    assert!(check.ok);
    assert_eq!(check.table_count, 1);
    assert!(!check.schema_fingerprint.is_empty());
}

#[test]
fn check_schema_source_rejects_invalid_document() {
    let check = check_schema_source("not a schema");
    assert!(!check.ok);
    assert_eq!(check.table_count, 0);
    assert!(check.schema_fingerprint.is_empty());
    assert!(check.error.is_some());
}

#[test]
fn introspect_schema_lists_table_fields() {
    let payload = introspect_schema_json(USER_SCHEMA);
    let parsed = parse_json(&payload);
    assert_eq!(parsed.get("ok"), Some(&JsonValue::Bool(true)));
    assert!(parsed.get("engineVersion").is_none());
    let tables = parsed.get("tables").and_then(JsonValue::as_array).expect("tables");
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].get("name"), Some(&JsonValue::String("User".into())));
}
