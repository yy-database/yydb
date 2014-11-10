//! Host-testable WASM shared surface (no wasm-bindgen).

use serde_json::{Value as JsonValue, json};
use yydb::{
    Connection, Error, Result, SchemaVersion, Value,
    catalog::{self, ExecutionTypeKind},
};

/// Build the full diagnostic contract JSON object for any error branch.
///
/// All `*_json` WASM helpers embed this as the `"error"` field on failed
/// outcomes, so browser hosts receive `{ code, operation, retryable, message,
/// params? }` instead of a flattened display string.
fn err_diag(err: &Error) -> JsonValue {
    err.to_json()
}

/// Outcome of validating a VOS schema document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaCheck {
    /// Whether the document parsed and lowered into the execution slice.
    pub ok: bool,
    /// Number of table types in the lowered catalog.
    pub table_count: u32,
    /// Stable document fingerprint (BLAKE3 hex, first 16 chars).
    pub schema_fingerprint: String,
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
    match catalog::execution_catalog(source) {
        Ok(catalog) => {
            let table_count =
                u32::try_from(catalog.types.iter().filter(|entry| entry.kind == ExecutionTypeKind::Table).count())
                    .unwrap_or(u32::MAX);
            SchemaCheck { ok: true, table_count, schema_fingerprint: schema_fingerprint(source), error: None }
        }
        Err(err) => SchemaCheck { ok: false, table_count: 0, schema_fingerprint: String::new(), error: Some(err.to_string()) },
    }
}

/// Read-only schema introspection JSON.
pub fn introspect_schema_json(source: &str) -> String {
    match catalog::execution_catalog(source) {
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
                "schemaFingerprint": schema_fingerprint(source),
                "tables": tables,
                "error": JsonValue::Null,
            })
            .to_string()
        }
        Err(err) => json!({
            "ok": false,
            "schemaFingerprint": "",
            "tables": [],
            "error": err_diag(&err),
        })
        .to_string(),
    }
}

pub(crate) fn unit_result_json(result: Result<()>) -> String {
    match result {
        Ok(()) => json!({
            "ok": true,
            "rows": [],
            "error": JsonValue::Null,
        })
        .to_string(),
        Err(err) => json!({
            "ok": false,
            "rows": [],
            "error": err_diag(&err),
        })
        .to_string(),
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
            "error": err_diag(&err),
        })
        .to_string(),
    }
}

/// Build the same info text as the YY wire `Info` handler.
pub(crate) fn connection_info(conn: &Connection, path: Option<&str>) -> String {
    let path = path
        .map(str::to_owned)
        .or_else(|| conn.path().map(|entry| entry.display().to_string()))
        .unwrap_or_else(|| "<memory>".to_owned());
    let schema_line = match conn.schema() {
        Ok(Some(schema)) => format!("schema.version={}", schema.version),
        Ok(None) => "schema.version=".to_owned(),
        Err(_) => "schema.version=".to_owned(),
    };
    let records = conn.record_count().unwrap_or(0);
    format!("path={path}\n{schema_line}\nrecords={records}\njournal_mode={}\n", conn.journal_mode().as_str())
}

pub(crate) fn scalar_call_json(result: Result<Value>) -> String {
    match result {
        Ok(value) => match scalar_value_json(&value) {
            Ok(scalar) => json!({
                "ok": true,
                "value": scalar,
                "error": JsonValue::Null,
            })
            .to_string(),
            Err(err) => json!({
                "ok": false,
                "value": JsonValue::Null,
                "error": err_diag(&err),
            })
            .to_string(),
        },
        Err(err) => json!({
            "ok": false,
            "value": JsonValue::Null,
            "error": err_diag(&err),
        })
        .to_string(),
    }
}

fn scalar_value_json(value: &Value) -> Result<JsonValue> {
    match value {
        Value::Null => Ok(JsonValue::Null),
        Value::Bool(value) => Ok(json!(value)),
        Value::I64(value) => Ok(json!(value)),
        Value::Text(value) => Ok(json!(value)),
        _ => Err(yydb::Error::Unsupported("scalar call value type is outside the Phase-1 wire subset")),
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
            "error": err_diag(&err),
        })
        .to_string(),
    }
}

pub(crate) fn scan_prefix_json(result: Result<Vec<(String, Vec<u8>)>>) -> String {
    match result {
        Ok(rows) => json!({
            "ok": true,
            "rows": rows.into_iter().map(|(k, v)| json!({ "key": k, "value": v })).collect::<Vec<JsonValue>>(),
            "error": JsonValue::Null,
        })
        .to_string(),
        Err(err) => json!({
            "ok": false,
            "rows": [],
            "error": err_diag(&err),
        })
        .to_string(),
    }
}

pub(crate) fn compare_exchange_json(result: Result<bool>) -> String {
    match result {
        Ok(success) => json!({
            "ok": true,
            "success": success,
            "error": JsonValue::Null,
        })
        .to_string(),
        Err(err) => json!({
            "ok": false,
            "success": false,
            "error": err_diag(&err),
        })
        .to_string(),
    }
}

pub(crate) fn query_rows_json(result: Result<Vec<yydb_query::QueryRow>>) -> String {
    match result {
        Ok(rows) => json!({
            "ok": true,
            "rows": rows.into_iter().map(|row| {
                let mut map = serde_json::Map::new();
                for (k, v) in row {
                    map.insert(k, scalar_value_json(&v).unwrap_or(JsonValue::Null));
                }
                JsonValue::Object(map)
            }).collect::<Vec<JsonValue>>(),
            "error": JsonValue::Null,
        })
        .to_string(),
        Err(err) => json!({
            "ok": false,
            "rows": [],
            "error": err_diag(&err),
        })
        .to_string(),
    }
}
