//! Table row persistence inside the YYDB record map.

use std::collections::BTreeMap;

use serde_json::{Map, Value as JsonValue};
use yydb_types::{Error, Result, Value};

use super::ops::QueryRow;

const ROW_PREFIX: &str = "__yydb/v1/row/";

/// Build the storage key for one table row.
pub fn row_key(table: &str, pk: &str) -> String {
    format!("{ROW_PREFIX}{table}/{pk}")
}

fn table_prefix(table: &str) -> String {
    format!("{ROW_PREFIX}{table}/")
}

/// Load all rows for `table` from the record map.
pub fn load_table(records: &BTreeMap<String, Vec<u8>>, table: &str) -> Result<Vec<QueryRow>> {
    let prefix = table_prefix(table);
    let mut out = Vec::new();
    for (key, bytes) in records.range(prefix.clone()..) {
        if !key.starts_with(&prefix) {
            break;
        }
        out.push(decode_row(bytes)?);
    }
    Ok(out)
}

/// Upsert one logical row keyed by `pk`.
pub fn upsert_row(
    records: &mut BTreeMap<String, Vec<u8>>,
    table: &str,
    pk: &str,
    row: &QueryRow,
) -> Result<()> {
    records.insert(row_key(table, pk), encode_row(row)?);
    Ok(())
}

fn encode_row(row: &QueryRow) -> Result<Vec<u8>> {
    let mut map = Map::new();
    for (key, value) in row {
        map.insert(key.clone(), encode_value(value)?);
    }
    serde_json::to_vec(&JsonValue::Object(map))
        .map_err(|_| Error::Corrupt("row json encode failed"))
}

/// Load one row by primary key.
pub fn load_row(records: &BTreeMap<String, Vec<u8>>, table: &str, pk: &str) -> Result<QueryRow> {
    let key = row_key(table, pk);
    let bytes = records.get(&key).ok_or_else(|| Error::Schema {
        message: format!("missing row `{table}` pk `{pk}`"),
    })?;
    decode_row(bytes)
}

fn decode_row(bytes: &[u8]) -> Result<QueryRow> {
    let json: JsonValue =
        serde_json::from_slice(bytes).map_err(|_| Error::Corrupt("row json decode failed"))?;
    let object = json
        .as_object()
        .ok_or(Error::Corrupt("row json must be an object"))?;
    let mut row = QueryRow::new();
    for (key, value) in object {
        row.insert(key.clone(), decode_value(value)?);
    }
    Ok(row)
}

fn encode_value(value: &Value) -> Result<JsonValue> {
    Ok(match value {
        Value::Null => JsonValue::Null,
        Value::Bool(b) => JsonValue::Bool(*b),
        Value::I64(i) => JsonValue::from(*i),
        Value::Text(s) => JsonValue::String(s.clone()),
        Value::Uuid(id) => JsonValue::String(id.to_string()),
        _ => {
            return Err(Error::Unsupported(
                "query row encoding supports null, bool, i64, text, and uuid only",
            ));
        }
    })
}

fn decode_value(value: &JsonValue) -> Result<Value> {
    Ok(match value {
        JsonValue::Null => Value::Null,
        JsonValue::Bool(b) => Value::Bool(*b),
        JsonValue::Number(n) => Value::I64(n.as_i64().unwrap_or(0)),
        JsonValue::String(s) => Value::Text(s.clone()),
        _ => {
            return Err(Error::Unsupported(
                "query row decoding supports null, bool, number, and string only",
            ));
        }
    })
}
