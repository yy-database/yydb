//! Stateless schema validation and introspection (no build version in payloads).

use crate::fixtures::{USER_SCHEMA, parse_json};
use serde_json::Value as JsonValue;
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
    let check = check_schema_source("table User { @@id: i64 } ]");
    assert!(!check.ok);
    assert_eq!(check.table_count, 0);
    assert!(check.schema_fingerprint.is_empty());
    assert!(check.error.is_some());
}

#[test]
fn introspection_reports_exact_scalar_types_and_unique_constraints() {
    let source = r#"
table Event {
    @@event_id: i64,
    @happened_on: date,
    at_time: time,
    created_at: datetime,
    amount: Decimal<38, 2>,
}
"#;
    let parsed = parse_json(&introspect_schema_json(source));
    let tables = parsed.get("tables").and_then(JsonValue::as_array).expect("tables");
    let table = &tables[0];
    let fields = table.get("fields").and_then(JsonValue::as_array).expect("fields");
    let field = |name: &str| {
        fields
            .iter()
            .find(|field| field.get("name") == Some(&JsonValue::String(name.into())))
            .expect("field present")
            .get("type")
            .cloned()
            .expect("type descriptor")
    };
    assert_eq!(field("happened_on"), serde_json::json!({ "kind": "date" }));
    assert_eq!(field("at_time"), serde_json::json!({ "kind": "time", "unit": "nanosecond", "timezone": "none" }));
    assert_eq!(field("created_at"), serde_json::json!({ "kind": "timestamp", "unit": "microsecond", "timezone": "utc" }));
    assert_eq!(field("amount"), serde_json::json!({ "kind": "decimal", "precision": 38, "scale": 2 }));

    let constraints = table.get("uniqueConstraints").and_then(JsonValue::as_array).expect("uniqueConstraints");
    assert_eq!(constraints.len(), 1);
    assert_eq!(constraints[0].get("name"), Some(&JsonValue::String("Event_happened_on_key".into())));
    assert_eq!(constraints[0].get("columns"), Some(&serde_json::json!(["happened_on"])));
    assert_eq!(constraints[0].get("nullsDistinct"), Some(&JsonValue::Bool(true)));
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
