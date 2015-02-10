//! Execute backend-neutral write operators against stored table rows.
//!
//! Every write frontend lowers into [`WriteOp`]; this module resolves those
//! ops against the catalog, enforces uniqueness rules and mutates the record
//! map. Like the read pipeline it never sees VOS or SQL syntax.

use std::collections::BTreeMap;

use vos::types::{ResolvedContract, ResolvedTypeContract};
use yydb_types::{Error, Result, TableUniqueConstraint, Value};

use super::{
    catalog,
    coerce,
    constraints,
    exec,
    ops::{BoundValue, Pred, QueryRow, WriteAssignment, WriteField, WriteOp},
    store,
};

fn write_error(message: impl Into<String>) -> Error {
    Error::Schema { message: message.into() }
}

/// What a write program changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WriteOutcome {
    /// Number of rows inserted, updated or deleted.
    pub affected: usize,
    /// Rows inserted or replaced, in statement order.
    pub rows: Vec<QueryRow>,
}

/// Execute a write program in order, returning the accumulated outcome.
pub fn execute_ops(
    ops: &[WriteOp],
    contract: &ResolvedContract,
    declared: &[TableUniqueConstraint],
    records: &mut BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
) -> Result<WriteOutcome> {
    let mut outcome = WriteOutcome::default();
    for op in ops {
        match op {
            WriteOp::Insert { table, fields } => {
                let row = insert_row(table, fields, contract, declared, records, bound_values)?;
                outcome.rows.push(row);
                outcome.affected += 1;
            }
            WriteOp::Update { table, assignments, predicate } => {
                outcome.affected +=
                    update_rows(table, assignments, predicate.as_ref(), contract, declared, records, bound_values)?;
            }
            WriteOp::Delete { table, predicate } => {
                outcome.affected += delete_rows(table, predicate.as_ref(), contract, records, bound_values)?;
            }
        }
    }
    Ok(outcome)
}

fn resolve_table<'a>(contract: &'a ResolvedContract, table: &str) -> Result<(&'a ResolvedTypeContract, String)> {
    let entry = catalog::table_type_by_name(contract, table)?;
    let storage = catalog::table_storage_name(entry)?.to_string();
    Ok((entry, storage))
}

fn resolve_value<'a>(bound: &'a BoundValue, bound_values: &'a [Value]) -> Result<&'a Value> {
    match bound {
        BoundValue::Literal(value) => Ok(value),
        BoundValue::Slot { index, .. } => bound_values
            .get(*index as usize)
            .ok_or_else(|| write_error(format!("bound parameter index {index} out of range"))),
    }
}

fn insert_row(
    table: &str,
    fields: &[WriteField],
    contract: &ResolvedContract,
    declared: &[TableUniqueConstraint],
    records: &mut BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
) -> Result<QueryRow> {
    let (entry, storage) = resolve_table(contract, table)?;
    let mut row = QueryRow::new();
    for field in fields {
        let slot = catalog::field_on_table(entry, &field.name)?;
        let value = resolve_value(&field.value, bound_values)?;
        row.insert(field.name.clone(), coerce::value_to_scalar(value.clone(), &slot.canonical_type)?);
    }
    let pk_field = catalog::primary_field_name(entry)?;
    let pk = row
        .get(pk_field)
        .ok_or_else(|| write_error(format!("insert into `{table}` is missing primary key field `{pk_field}`")))?;
    let pk_text = constraints::value_text(pk)?;
    constraints::check(records, declared, &storage, pk_field, &pk_text, &row)?;
    store::upsert_row(records, &storage, &pk_text, &row)?;
    Ok(row)
}

fn update_rows(
    table: &str,
    assignments: &[WriteAssignment],
    predicate: Option<&Pred>,
    contract: &ResolvedContract,
    declared: &[TableUniqueConstraint],
    records: &mut BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
) -> Result<usize> {
    let (entry, storage) = resolve_table(contract, table)?;
    let pk_field = catalog::primary_field_name(entry)?.to_string();

    let mut replacements: Vec<(String, QueryRow)> = Vec::new();
    {
        let entries = store::load_table_entries(records, &storage)?;
        for (key, mut row) in entries {
            if let Some(predicate) = predicate {
                if !exec::eval_pred(predicate, &row, table, None, records, bound_values) {
                    continue;
                }
            }
            for assignment in assignments {
                let slot = catalog::field_on_table(entry, &assignment.field)?;
                if assignment.field == pk_field {
                    return Err(write_error(format!("primary key field `{pk_field}` cannot be updated")));
                }
                let value = resolve_value(&assignment.value, bound_values)?;
                row.insert(assignment.field.clone(), coerce::value_to_scalar(value.clone(), &slot.canonical_type)?);
            }
            replacements.push((key, row));
        }
    }

    for (key, row) in &replacements {
        let pk = row
            .get(&pk_field)
            .ok_or_else(|| write_error(format!("updated `{table}` row lost primary key field `{pk_field}`")))?;
        let pk_text = constraints::value_text(pk)?;
        constraints::check(records, declared, &storage, &pk_field, &pk_text, row)?;
        store::remove_row(records, key);
        store::upsert_row(records, &storage, &pk_text, row)?;
    }
    Ok(replacements.len())
}

fn delete_rows(
    table: &str,
    predicate: Option<&Pred>,
    contract: &ResolvedContract,
    records: &mut BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
) -> Result<usize> {
    let (_, storage) = resolve_table(contract, table)?;
    let mut doomed: Vec<String> = Vec::new();
    {
        let entries = store::load_table_entries(records, &storage)?;
        for (key, row) in entries {
            if let Some(predicate) = predicate {
                if !exec::eval_pred(predicate, &row, table, None, records, bound_values) {
                    continue;
                }
            }
            doomed.push(key);
        }
    }
    let mut removed = 0;
    for key in doomed {
        if store::remove_row(records, &key) {
            removed += 1;
        }
    }
    Ok(removed)
}
