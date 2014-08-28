//! Resolve dotted field paths against stored rows and the VOS schema projection.

use std::collections::BTreeMap;

use vos::types::{CanonicalType, FieldContract, SchemaProjection, TypeContract, TypeContractKind};

use yydb_types::{Error, Result, Value};

use super::ops::QueryRow;
use super::store;

fn resolve_error(message: impl Into<String>) -> Error {
    Error::Schema {
        message: message.into(),
    }
}

fn table_name(entry: &TypeContract) -> String {
    entry.canonical_path.join("::")
}

fn table_entry<'a>(catalog: &'a SchemaProjection, name: &str) -> Result<&'a TypeContract> {
    catalog
        .types
        .iter()
        .find(|entry| {
            entry.kind == TypeContractKind::Table
                && entry.canonical_path.last().is_some_and(|last| last == name)
        })
        .ok_or_else(|| resolve_error(format!("unknown table `{name}`")))
}

fn field_entry<'a>(entry: &'a TypeContract, name: &str) -> Option<&'a FieldContract> {
    entry
        .fields
        .iter()
        .find(|field| field.canonical_name == name)
}

fn reference_target(ty: &CanonicalType) -> Result<&str> {
    match ty {
        CanonicalType::Reference(inner) => match inner.as_ref() {
            CanonicalType::Named(path) => path
                .last()
                .map(String::as_str)
                .ok_or_else(|| resolve_error("reference field must name a table type")),
            _ => Err(resolve_error("reference field must name a table type")),
        },
        _ => Err(resolve_error("field is not a reference")),
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
    catalog: &SchemaProjection,
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

        let entry = table_entry(catalog, &current_table)?;
        let field = field_entry(entry, segment).ok_or_else(|| {
            resolve_error(format!(
                "unknown field `{segment}` on table `{}`",
                table_name(entry)
            ))
        })?;
        let target_table = reference_target(&field.canonical_type)?.to_string();
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
    catalog: &SchemaProjection,
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
        let entry = table_entry(catalog, &current_table)?;
        let field = field_entry(entry, segment).ok_or_else(|| {
            resolve_error(format!(
                "unknown field `{segment}` on table `{}`",
                table_name(entry)
            ))
        })?;
        let target_table = reference_target(&field.canonical_type)?.to_string();
        let pk = value_to_pk(&value)?;
        owned = Some(store::load_row(records, &target_table, &pk)?);
        current = owned.as_ref().expect("loaded referenced row");
        current_table = target_table;
    }

    Ok((
        owned.unwrap_or_else(|| row.clone()),
        current_table.to_string(),
    ))
}
