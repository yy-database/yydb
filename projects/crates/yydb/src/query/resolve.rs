//! Resolve dotted field paths against stored rows and the VOS catalog.

use std::collections::BTreeMap;

use vos::ast::TypeExpr;
use vos::ast::catalog::{CatalogSnapshot, TypeEntry, TypeKind};

use yydb_types::{Error, Result, Value};

use super::ops::QueryRow;
use super::store;

fn resolve_error(message: impl Into<String>) -> Error {
    Error::Schema {
        message: message.into(),
    }
}

fn table_entry<'a>(catalog: &'a CatalogSnapshot, name: &str) -> Result<&'a TypeEntry> {
    catalog
        .types
        .iter()
        .find(|entry| entry.name == name && entry.kind == TypeKind::Table)
        .ok_or_else(|| resolve_error(format!("unknown table `{name}`")))
}

fn reference_target(ty: &TypeExpr) -> Result<&str> {
    match ty {
        TypeExpr::Reference(inner) => match inner.as_ref() {
            TypeExpr::Named(name) => Ok(name.as_str()),
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
        _ => return Err(resolve_error("reference key must be text, uuid, integer, or bool")),
    })
}

/// Read one field path from a row, dereferencing `&Table` hops via `records`.
pub fn resolve_field_path(
    path: &[String],
    row: &QueryRow,
    table: &str,
    catalog: &CatalogSnapshot,
    records: &BTreeMap<String, Vec<u8>>,
) -> Result<Value> {
    if path.is_empty() {
        return Ok(Value::Null);
    }
    if path.len() == 1 {
        return Ok(row.get(&path[0]).cloned().unwrap_or(Value::Null));
    }

    let mut current_table = table;
    let mut owned: Option<QueryRow> = None;
    let mut current: &QueryRow = row;

    for (index, segment) in path.iter().enumerate() {
        let value = current
            .get(segment)
            .cloned()
            .unwrap_or(Value::Null);
        if index + 1 == path.len() {
            return Ok(value);
        }

        let entry = table_entry(catalog, current_table)?;
        let field = entry
            .fields
            .iter()
            .find(|field| field.current_name == *segment)
            .ok_or_else(|| {
                resolve_error(format!(
                    "unknown field `{segment}` on table `{current_table}`"
                ))
            })?;
        let target_table = reference_target(&field.ty)?;
        let pk = value_to_pk(&value)?;
        owned = Some(store::load_row(records, target_table, &pk)?);
        current = owned.as_ref().expect("loaded referenced row");
        current_table = target_table;
    }

    Ok(Value::Null)
}
