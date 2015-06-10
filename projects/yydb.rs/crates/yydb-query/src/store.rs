//! Table row persistence inside the YYDB record map.

use std::collections::BTreeMap;

use serde_json::{Map, Value as JsonValue};
use yydb_types::{Error, Result, Value, scalar};

use super::ops::QueryRow;

const ROW_PREFIX: &str = "__yydb/v0/row/";

/// Build the storage key for one table row.
pub fn row_key(table: &str, pk: &str) -> String {
    format!("{ROW_PREFIX}{table}/{pk}")
}

fn table_prefix(table: &str) -> String {
    format!("{ROW_PREFIX}{table}/")
}

/// Load all rows for `table` from the record map.
pub fn load_table(records: &BTreeMap<String, Vec<u8>>, table: &str) -> Result<Vec<QueryRow>> {
    Ok(load_table_entries(records, table)?.into_iter().map(|(_, row)| row).collect())
}

/// Load all rows for `table` together with their record-map keys.
///
/// Write operators need the key to delete or replace a row in place, so this
/// variant is the shared primitive and [`load_table`] is the read-only view.
pub fn load_table_entries(records: &BTreeMap<String, Vec<u8>>, table: &str) -> Result<Vec<(String, QueryRow)>> {
    let prefix = table_prefix(table);
    let mut out = Vec::new();
    for (key, bytes) in records.range(prefix.clone()..) {
        if !key.starts_with(&prefix) {
            break;
        }
        out.push((key.clone(), decode_row(bytes)?));
    }
    Ok(out)
}

/// Remove the row stored under `key`. Returns whether a row was present.
pub fn remove_row(records: &mut BTreeMap<String, Vec<u8>>, key: &str) -> bool {
    records.remove(key).is_some()
}

/// Upsert one logical row keyed by `pk`.
pub fn upsert_row(records: &mut BTreeMap<String, Vec<u8>>, table: &str, pk: &str, row: &QueryRow) -> Result<()> {
    records.insert(row_key(table, pk), encode_row(row)?);
    Ok(())
}

fn encode_row(row: &QueryRow) -> Result<Vec<u8>> {
    let mut map = Map::new();
    for (key, value) in row {
        map.insert(key.clone(), encode_value(value)?);
    }
    serde_json::to_vec(&JsonValue::Object(map)).map_err(|_| Error::Corrupt("row json encode failed"))
}

/// Load one row by primary key.
pub fn load_row(records: &BTreeMap<String, Vec<u8>>, table: &str, pk: &str) -> Result<QueryRow> {
    let key = row_key(table, pk);
    let bytes = records.get(&key).ok_or_else(|| Error::Schema { message: format!("missing row `{table}` pk `{pk}`") })?;
    decode_row(bytes)
}

fn decode_row(bytes: &[u8]) -> Result<QueryRow> {
    let json: JsonValue = serde_json::from_slice(bytes).map_err(|_| Error::Corrupt("row json decode failed"))?;
    let object = json.as_object().ok_or(Error::Corrupt("row json must be an object"))?;
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
        Value::F64(number) => {
            if !number.is_finite() {
                return Err(Error::Unsupported("query row encoding rejects non-finite REAL values"));
            }
            JsonValue::Number(
                serde_json::Number::from_f64(*number)
                    .ok_or(Error::Unsupported("query row encoding rejects non-finite REAL values"))?,
            )
        }
        Value::Text(s) => JsonValue::String(s.clone()),
        Value::Uuid(id) => JsonValue::String(id.to_string()),
        Value::Date(_) | Value::Time(_) | Value::Timestamp(_) | Value::Decimal(_) | Value::Bytes(_) => {
            scalar::tagged_value_json(value).expect("temporal, decimal, and binary values always have a tagged form")
        }
        _ => {
            return Err(Error::Unsupported(
                "query row encoding supports null, bool, i64, real, text, uuid, bytes, date, time, timestamp, and decimal \
                 only",
            ));
        }
    })
}

fn decode_value(value: &JsonValue) -> Result<Value> {
    Ok(match value {
        JsonValue::Null => Value::Null,
        JsonValue::Bool(b) => Value::Bool(*b),
        JsonValue::Number(n) => {
            if let Some(integer) = n.as_i64() {
                Value::I64(integer)
            }
            else if n.is_f64() {
                let real = n.as_f64().ok_or_else(|| Error::Corrupt("row json real is not a finite f64"))?;
                if !real.is_finite() {
                    return Err(Error::Corrupt("row json real is not finite"));
                }
                Value::F64(real)
            }
            else {
                return Err(Error::Corrupt("row json integer is outside the signed 64-bit range"));
            }
        }
        JsonValue::String(s) => Value::Text(s.clone()),
        JsonValue::Object(_) => match scalar::tagged_value_from_json(value) {
            Some(Ok(value)) => value,
            Some(Err(error)) => return Err(error.into()),
            None => {
                return Err(Error::Unsupported(
                    "query row decoding supports tagged date, time, timestamp, decimal, and bytes objects only",
                ));
            }
        },
        JsonValue::Array(_) => {
            return Err(Error::Unsupported("query row decoding does not support JSON arrays"));
        }
    })
}
