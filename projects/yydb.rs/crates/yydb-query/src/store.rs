//! Table row persistence inside the YYDB record map.

use std::{cell::RefCell, collections::BTreeMap, sync::Arc};

use serde_json::{Map, Value as JsonValue};
use yydb_types::{Error, ObjectRef, Result, Value, scalar};

use super::ops::QueryRow;

const ROW_PREFIX: &str = "__yydb/v0/row/";

/// Reads the octets behind an out-of-line `bytes` object reference.
///
/// The storage host owns placement, so the row codec asks it to resolve a
/// reference instead of knowing how objects are kept.
pub trait ObjectResolver: Send + Sync {
    /// Returns the stored octets, failing closed when the object is missing or corrupt.
    fn resolve(&self, object: &ObjectRef) -> Result<Vec<u8>>;
}

type SharedResolver = Arc<dyn ObjectResolver>;

/// Storage hook that decides where a written `bytes` value is kept.
///
/// The octet threshold is a physical property of the host, so the query layer
/// never picks one: the host installs a hook and receives either the original
/// inline octets or an object reference it created itself.
pub trait BytesPlacementHook: Send + Sync {
    /// Returns the value to store: inline octets or a reference into the host object store.
    fn place(&self, octets: Vec<u8>) -> Result<Value>;
}

type SharedPlacementHook = Arc<dyn BytesPlacementHook>;

thread_local! {
    /// Resolver installed for the duration of one statement execution on this thread.
    static OBJECT_RESOLVER: RefCell<Option<SharedResolver>> = RefCell::new(None);
    /// Placement hook installed for the duration of one write execution on this thread.
    static BYTES_PLACEMENT: RefCell<Option<SharedPlacementHook>> = RefCell::new(None);
}

struct ResolverGuard(Option<SharedResolver>);

impl Drop for ResolverGuard {
    fn drop(&mut self) {
        let previous = self.0.take();
        OBJECT_RESOLVER.with(|slot| {
            slot.replace(previous);
        });
    }
}

/// Runs `body` with `resolver` installed for row decoding on this thread.
///
/// Rows whose `bytes` values were placed in the host object store cannot be
/// decoded without a resolver, so a host that stores references always installs
/// one. Without it, decoding a reference returns [`Error::Unsupported`] instead
/// of guessing or downgrading the value to text.
pub fn with_object_resolver<T>(resolver: Option<SharedResolver>, body: impl FnOnce() -> T) -> T {
    let previous = OBJECT_RESOLVER.with(|slot| slot.replace(resolver));
    let _guard = ResolverGuard(previous);
    body()
}

struct PlacementGuard(Option<SharedPlacementHook>);

impl Drop for PlacementGuard {
    fn drop(&mut self) {
        let previous = self.0.take();
        BYTES_PLACEMENT.with(|slot| {
            slot.replace(previous);
        });
    }
}

/// Runs `body` with `hook` installed for `bytes` placement on this thread.
///
/// Without a hook every `bytes` value stays inline, which is what a host with
/// no object store can actually keep.
pub fn with_bytes_placement<T>(hook: Option<SharedPlacementHook>, body: impl FnOnce() -> T) -> T {
    let previous = BYTES_PLACEMENT.with(|slot| slot.replace(hook));
    let _guard = PlacementGuard(previous);
    body()
}

/// Places one `bytes` value through the installed hook.
pub(crate) fn place_bytes(octets: Vec<u8>) -> Result<Value> {
    BYTES_PLACEMENT.with(|slot| match slot.borrow().as_ref() {
        Some(hook) => hook.place(octets),
        None => Ok(Value::Bytes(octets)),
    })
}

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
        Value::Uuid(_)
        | Value::Date(_)
        | Value::Time(_)
        | Value::Timestamp(_)
        | Value::Decimal(_)
        | Value::Bytes(_)
        | Value::Vector(_)
        | Value::Object(_) => scalar::tagged_value_json(value)
            .expect("uuid, temporal, decimal, binary, vector, and object-reference values always have a tagged form"),
        _ => {
            return Err(Error::Unsupported(
                "query row encoding supports null, bool, i64, real, text, uuid, bytes, vector, date, time, timestamp, \
                 decimal, and bytes object references only",
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
            Some(Ok(parsed)) => resolve_object_reference(parsed)?,
            Some(Err(error)) => return Err(error.into()),
            None => {
                return Err(Error::Unsupported(
                    "query row decoding supports tagged uuid, date, time, timestamp, decimal, bytes, vector, and object \
                     references only",
                ));
            }
        },
        JsonValue::Array(_) => {
            return Err(Error::Unsupported("query row decoding does not support JSON arrays"));
        }
    })
}

fn resolve_object_reference(value: Value) -> Result<Value> {
    match value {
        Value::Object(object) => OBJECT_RESOLVER.with(|slot| match slot.borrow().as_ref() {
            Some(resolver) => resolver.resolve(&object).map(Value::Bytes),
            None => Err(Error::Unsupported("a stored bytes object reference needs an object store to decode")),
        }),
        other => Ok(other),
    }
}
