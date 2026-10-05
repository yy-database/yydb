//! Host-testable WASM shared surface (no wasm-bindgen).

use std::collections::BTreeMap;

use serde_json::{json, Map, Value as JsonValue};
use yydb::{
    schema::{self, ExecutionTypeKind},
    Result, SchemaVersion, Value,
};

/// Outcome of validating a VOS schema document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaCheck {
    /// Whether the document parsed and lowered into the execution slice.
    pub ok: bool,
    /// Number of table types in the lowered catalog.
    pub table_count: u32,
    /// Stable document fingerprint (BLAKE3 hex, first 16 chars).
    pub schema_fingerprint: String,
    /// Engine version string (`yydb::version()` / workspace semver).
    pub engine_version: String,
    /// Structured error when [`Self::ok`] is false.
    pub error: Option<String>,
}

/// Crate version (matches `yydb::version()` / workspace semver).
pub fn yydb_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn schema_fingerprint(document: &str) -> String {
    let hash = blake3::hash(document.as_bytes()).to_hex();
    hash[..16.min(hash.len())].to_string()
}

/// Parse and validate schema source (same semantics as `Connection::ensure_schema`).
pub fn check_schema_source(source: &str) -> SchemaCheck {
    match schema::execution_catalog(source) {
        Ok(catalog) => {
            let table_count = u32::try_from(
                catalog
                    .types
                    .iter()
                    .filter(|entry| entry.kind == ExecutionTypeKind::Table)
                    .count(),
            )
            .unwrap_or(u32::MAX);
            SchemaCheck {
                ok: true,
                table_count,
                schema_fingerprint: schema_fingerprint(source),
                engine_version: yydb_version(),
                error: None,
            }
        }
        Err(err) => SchemaCheck {
            ok: false,
            table_count: 0,
            schema_fingerprint: String::new(),
            engine_version: yydb_version(),
            error: Some(err.to_string()),
        },
    }
}

/// Read-only schema introspection JSON.
pub fn introspect_schema_json(source: &str) -> String {
    match schema::execution_catalog(source) {
        Ok(catalog) => {
            let tables = catalog
                .types
                .iter()
                .filter(|entry| entry.kind == ExecutionTypeKind::Table)
                .map(|entry| {
                    json!({
                        "name": entry.name,
                        "schemaId": entry.schema_id,
                        "fields": entry.fields.iter().map(|field| {
                            json!({
                                "name": field.name,
                                "fieldId": field.field_id,
                                "virtualField": field.virtual_field,
                            })
                        }).collect::<Vec<JsonValue>>(),
                    })
                })
                .collect::<Vec<JsonValue>>();
            json!({
                "ok": true,
                "engineVersion": yydb_version(),
                "schemaFingerprint": schema_fingerprint(source),
                "tables": tables,
                "error": JsonValue::Null,
            })
            .to_string()
        }
        Err(err) => json!({
            "ok": false,
            "engineVersion": yydb_version(),
            "schemaFingerprint": "",
            "tables": [],
            "error": err.to_string(),
        })
        .to_string(),
    }
}

pub(crate) fn rows_to_json(rows: Vec<BTreeMap<String, Value>>) -> Vec<JsonValue> {
    rows.into_iter()
        .map(|row| {
            let mut obj = Map::new();
            for (key, value) in row {
                obj.insert(key, value_to_json(&value));
            }
            JsonValue::Object(obj)
        })
        .collect()
}

#[allow(clippy::wildcard_enum_match_arm)]
pub(crate) fn value_to_json(value: &Value) -> JsonValue {
    match value {
        Value::Null => JsonValue::Null,
        Value::Bool(b) => json!(b),
        Value::I64(n) => json!(n),
        Value::U64(n) => json!(n),
        Value::F64(n) => json!(n),
        Value::Text(s) => json!(s),
        Value::Bytes(bytes) => json!(bytes),
        Value::Uuid(id) => json!(id.to_string()),
        Value::Row(fields) => {
            let mut obj = Map::new();
            for (key, nested) in fields {
                obj.insert(key.clone(), value_to_json(nested));
            }
            JsonValue::Object(obj)
        }
        Value::Vector(vector) => json!({
            "kind": "vector",
            "dim": vector.dim,
            "data": vector.data.as_ref(),
        }),
        Value::Object(object) => json!({
            "kind": "object",
            "hash": object.hash_hex(),
            "size": object.size,
        }),
        Value::File(manifest) => json!({
            "kind": "file",
            "totalSize": manifest.total_size,
            "chunks": manifest.chunks.len(),
        }),
        _ => json!({ "kind": "unsupported" }),
    }
}

pub(crate) fn execute_result_json(result: Result<Vec<BTreeMap<String, Value>>>) -> String {
    match result {
        Ok(rows) => json!({
            "ok": true,
            "rows": rows_to_json(rows),
            "error": JsonValue::Null,
        })
        .to_string(),
        Err(err) => json!({
            "ok": false,
            "rows": [],
            "error": err.to_string(),
        })
        .to_string(),
    }
}

pub(crate) fn unit_result_json(result: Result<()>) -> String {
    match result {
        Ok(()) => json!({ "ok": true, "rows": [], "error": JsonValue::Null }).to_string(),
        Err(err) => json!({ "ok": false, "rows": [], "error": err.to_string() }).to_string(),
    }
}

pub(crate) fn kv_get_json(result: Result<Option<Vec<u8>>>) -> String {
    match result {
        Ok(Some(bytes)) => json!({
            "ok": true,
            "value": bytes,
            "error": JsonValue::Null,
        })
        .to_string(),
        Ok(None) => json!({
            "ok": true,
            "value": JsonValue::Null,
            "error": JsonValue::Null,
        })
        .to_string(),
        Err(err) => json!({
            "ok": false,
            "value": JsonValue::Null,
            "error": err.to_string(),
        })
        .to_string(),
    }
}

#[cfg(test)]
mod json_tests {
    use super::*;
    use yydb::Connection;

    #[test]
    fn kv_get_json_returns_bytes() {
        let conn = Connection::open_in_memory().expect("memory");
        conn.put("theme", b"dark").expect("put");
        let payload = kv_get_json(conn.get("theme"));
        let parsed: JsonValue = serde_json::from_str(&payload).expect("json");
        assert_eq!(parsed.get("ok"), Some(&JsonValue::Bool(true)));
        assert_eq!(parsed.get("value"), Some(&json!([100, 97, 114, 107])));
    }

    #[test]
    fn schema_get_json_empty() {
        let conn = Connection::open_in_memory().expect("memory");
        let payload = schema_get_json(conn.schema());
        let parsed: JsonValue = serde_json::from_str(&payload).expect("json");
        assert_eq!(parsed.get("ok"), Some(&JsonValue::Bool(true)));
        assert!(parsed.get("schema").unwrap().is_null());
    }
}

pub(crate) fn schema_get_json(result: Result<Option<SchemaVersion>>) -> String {
    match result {
        Ok(Some(schema)) => json!({
            "ok": true,
            "schema": {
                "version": schema.version,
                "document": schema.document,
            },
            "error": JsonValue::Null,
        })
        .to_string(),
        Ok(None) => json!({
            "ok": true,
            "schema": JsonValue::Null,
            "error": JsonValue::Null,
        })
        .to_string(),
        Err(err) => json!({
            "ok": false,
            "schema": JsonValue::Null,
            "error": err.to_string(),
        })
        .to_string(),
    }
}
