//! Resolve dotted field paths against stored rows and the resolved VOS contract.

use std::collections::BTreeMap;

use vos::types::ResolvedContract;

use yydb_types::{Error, Result, Value};

use super::catalog::{field_on_table, reference_target_storage_name, table_type_by_name};
use super::ops::QueryRow;
use super::store;

fn resolve_error(message: impl Into<String>) -> Error {
    Error::Schema {
        message: message.into(),
    }
}

fn value_to_pk(value: &Value) -> Result<String> {
    Ok(match value {
        Value::Text(text) => text.clone(),
        Value::Uuid(id) => id.to_string(),
        Value::I64(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Null => return Err(resolve_error("reference key cannot be null")),
        _ => {
            return Err(resolve_error(
                "reference key must be text, uuid, integer, or bool",
            ))
        }
    })
}

/// Read one field path from a row, dereferencing `&Table` hops via `records`.
pub fn resolve_field_path(
    path: &[String],
    row: &QueryRow,
    table: &str,
    contract: &ResolvedContract,
    records: &BTreeMap<String, Vec<u8>>,
) -> Result<Value> {
    if path.is_empty() {
        return Ok(Value::Null);
    }
    if path.len() == 1 {
        return Ok(row.get(&path[0]).cloned().unwrap_or(Value::Null));
    }

    let mut current_table = table.to_string();
    let mut owned: Option<QueryRow>;
    let mut current: &QueryRow = row;

    for (index, segment) in path.iter().enumerate() {
        let value = current.get(segment).cloned().unwrap_or(Value::Null);
        if index + 1 == path.len() {
            return Ok(value);
        }

        let entry = table_type_by_name(contract, &current_table)?;
        let field = field_on_table(entry, segment)?;
        let target_table = reference_target_storage_name(contract, &field.canonical_type)?;
        let pk = value_to_pk(&value)?;
        owned = Some(store::load_row(records, &target_table, &pk)?);
        current = owned.as_ref().expect("loaded referenced row");
        current_table = target_table;
    }

    Ok(Value::Null)
}

/// Load the row reached by dereferencing `path` from `row` (for nested projections).
pub fn resolve_row_at_path(
    path: &[String],
    row: &QueryRow,
    table: &str,
    contract: &ResolvedContract,
    records: &BTreeMap<String, Vec<u8>>,
) -> Result<(QueryRow, String)> {
    if path.is_empty() {
        return Ok((row.clone(), table.to_string()));
    }

    let mut current_table = table.to_string();
    let mut owned: Option<QueryRow> = None;
    let mut current: &QueryRow = row;

    for segment in path {
        let value = current.get(segment).cloned().unwrap_or(Value::Null);
        let entry = table_type_by_name(contract, &current_table)?;
        let field = field_on_table(entry, segment)?;
        let target_table = reference_target_storage_name(contract, &field.canonical_type)?;
        let pk = value_to_pk(&value)?;
        owned = Some(store::load_row(records, &target_table, &pk)?);
        current = owned.as_ref().expect("loaded referenced row");
        current_table = target_table;
    }

    Ok((
        owned.unwrap_or_else(|| row.clone()),
        current_table,
    ))
}
