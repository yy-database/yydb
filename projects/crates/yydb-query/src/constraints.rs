//! Uniqueness enforcement for the Phase 1 row store.
//!
//! Constraints come from two places: `@` (`unique`) attributes of the VOS
//! document and rules a host declares explicitly for composite column sets.
//! Both are expressed as [`TableUniqueConstraint`] so the write path has one
//! check. A conflict returns [`yydb_types::Error::UniqueViolation`] and leaves
//! the record map untouched.

use std::collections::BTreeMap;

use vos::types::{ResolvedContract, TypeContractKind};
use yydb_types::{Error, Result, TableUniqueConstraint, UniqueConstraint, Value};

use super::{catalog, ops::QueryRow, store};

/// Derives single-column unique constraints declared with the VOS `@` attribute.
pub fn from_contract(contract: &ResolvedContract) -> Result<Vec<TableUniqueConstraint>> {
    let mut constraints = Vec::new();
    for entry in contract.types.iter().filter(|entry| entry.kind == TypeContractKind::Table) {
        let table = catalog::table_storage_name(entry)?.to_string();
        let type_name = entry.canonical_path.last().cloned().unwrap_or_else(|| table.clone());
        for field in &entry.fields {
            if field.attributes.iter().any(|attribute| attribute.name == "unique") {
                let constraint = UniqueConstraint::new(
                    format!("{type_name}_{}_key", field.canonical_name),
                    vec![field.canonical_name.clone()],
                    true,
                )
                .map_err(Error::from)?;
                constraints.push(TableUniqueConstraint::new(table.clone(), constraint).map_err(Error::from)?);
            }
        }
    }
    Ok(constraints)
}

/// Combines document-derived rules with host declarations, deduplicated by
/// `(table, name)` with document rules taking precedence.
pub fn merge(
    document: Vec<TableUniqueConstraint>,
    declared: &[TableUniqueConstraint],
) -> Vec<TableUniqueConstraint> {
    let mut merged = document;
    for candidate in declared {
        let duplicate = merged
            .iter()
            .any(|existing| existing.table() == candidate.table() && existing.constraint().name() == candidate.constraint().name());
        if !duplicate {
            merged.push(candidate.clone());
        }
    }
    merged
}

/// Rejects `row` when it would duplicate an existing key of any constraint on `table`.
///
/// `row` replaces the row with `pk_text`, so that row never conflicts with itself.
/// NULL components follow the constraint's `nulls_distinct` flag and missing
/// fields count as NULL.
pub fn check(
    records: &BTreeMap<String, Vec<u8>>,
    constraints: &[TableUniqueConstraint],
    table: &str,
    pk_field: &str,
    pk_text: &str,
    row: &QueryRow,
) -> Result<()> {
    let applicable: Vec<&UniqueConstraint> = constraints
        .iter()
        .filter(|candidate| candidate.table() == table)
        .map(TableUniqueConstraint::constraint)
        .collect();
    if applicable.is_empty() {
        return Ok(());
    }

    let existing_rows = store::load_table(records, table)?;
    for constraint in applicable {
        let (incoming, incoming_null) = constraint_key(row, constraint.columns());
        if incoming_null && constraint.nulls_distinct() {
            continue;
        }
        for existing in &existing_rows {
            let existing_pk = existing
                .get(pk_field)
                .map(value_text)
                .transpose()?
                .unwrap_or_default();
            if existing_pk == pk_text {
                continue;
            }
            let (candidate, candidate_null) = constraint_key(existing, constraint.columns());
            if candidate_null && constraint.nulls_distinct() {
                continue;
            }
            if incoming == candidate {
                return Err(Error::UniqueViolation {
                    table: table.to_owned(),
                    constraint: constraint.name().to_owned(),
                    columns: constraint.columns().to_vec(),
                });
            }
        }
    }
    Ok(())
}

fn constraint_key(row: &QueryRow, columns: &[String]) -> (String, bool) {
    let mut parts = Vec::with_capacity(columns.len());
    let mut has_null = false;
    for column in columns {
        match row.get(column) {
            None | Some(Value::Null) => {
                has_null = true;
                parts.push(Value::Null.key_component());
            }
            Some(value) => parts.push(value.key_component()),
        }
    }
    (parts.join("\u{1f}"), has_null)
}

pub(crate) fn value_text(value: &Value) -> Result<String> {
    Ok(match value {
        Value::Text(text) => text.clone(),
        Value::Uuid(id) => id.to_string(),
        Value::I64(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Date(date) => date.to_iso(),
        Value::Time(time) => time.to_iso(),
        Value::Timestamp(timestamp) => timestamp.to_iso(),
        Value::Decimal(decimal) => decimal.to_plain_string(),
        Value::Null => return Err(Error::Schema { message: "primary key cannot be null".into() }),
        _ => return Err(Error::Schema { message: "primary key must be a scalar value".into() }),
    })
}
