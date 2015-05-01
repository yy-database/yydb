//! Execute backend-neutral write operators against stored table rows.
//!
//! Every write frontend lowers into [`WriteOp`]; this module resolves those
//! ops against the catalog, enforces uniqueness rules and mutates the record
//! map. Like the read pipeline it never sees VOS or SQL syntax.

use std::collections::BTreeMap;

use vos::types::{ResolvedContract, ResolvedTypeContract};
use yydb_types::{Error, Result, TableUniqueConstraint, Value};

use super::{
    catalog, coerce, constraints, exec,
    ops::{BoundValue, Pred, QueryOp, QueryRow, WriteAssignment, WriteBinOp, WriteExpr, WriteField, WriteOp},
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
            WriteOp::Insert { table, rows } => {
                let inserted = insert_rows(table, rows, contract, declared, records, bound_values)?;
                outcome.affected += inserted.len();
                outcome.rows.extend(inserted);
            }
            WriteOp::InsertFromQuery { table, columns, source, outputs } => {
                let inserted = insert_from_query(
                    table,
                    columns,
                    source,
                    outputs.as_deref(),
                    contract,
                    declared,
                    records,
                    bound_values,
                )?;
                outcome.affected += inserted.len();
                outcome.rows.extend(inserted);
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
        BoundValue::Slot { index, .. } => {
            bound_values.get(*index as usize).ok_or_else(|| write_error(format!("bound parameter index {index} out of range")))
        }
    }
}

/// Evaluate a [`WriteExpr`] against a single row (for `UPDATE … SET col = …`).
///
/// Unlike [`resolve_value`] this can read columns from the row being updated,
/// which is how the lowering of `score = score + 1` turns into a concrete
/// increment at execution time. `NULL` participating in arithmetic propagates
/// as `NULL` — three-valued semantics for write expressions.
fn eval_write_expr(expr: &WriteExpr, row: &QueryRow, bound_values: &[Value]) -> Result<Value> {
    match expr {
        WriteExpr::Bound(bound) => Ok(resolve_value(bound, bound_values)?.clone()),
        WriteExpr::Column { path } => {
            let Some(first) = path.first() else {
                return Err(write_error("empty column path in UPDATE expression"));
            };
            Ok(row.get(first).cloned().unwrap_or(Value::Null))
        }
        WriteExpr::Binary { op, left, right } => {
            let l = eval_write_expr(left, row, bound_values)?;
            let r = eval_write_expr(right, row, bound_values)?;
            if matches!(l, Value::Null) || matches!(r, Value::Null) {
                return Ok(Value::Null);
            }
            let (li64, ri64) = match (&l, &r) {
                (Value::I64(a), Value::I64(b)) => (*a, *b),
                (Value::I64(a), Value::U64(b)) => (*a, *b as i64),
                (Value::U64(a), Value::I64(b)) => (*a as i64, *b),
                (Value::U64(a), Value::U64(b)) => (*a as i64, *b as i64),
                (Value::F64(a), Value::F64(b)) => return Ok(Value::F64(eval_float_bin(*op, *a, *b)?)),
                (Value::F64(a), Value::I64(b)) => return Ok(Value::F64(eval_float_bin(*op, *a, *b as f64)?)),
                (Value::I64(a), Value::F64(b)) => return Ok(Value::F64(eval_float_bin(*op, *a as f64, *b)?)),
                (Value::F64(a), Value::U64(b)) => return Ok(Value::F64(eval_float_bin(*op, *a, *b as f64)?)),
                (Value::U64(a), Value::F64(b)) => return Ok(Value::F64(eval_float_bin(*op, *a as f64, *b)?)),
                _ => return Err(write_error(format!("UPDATE expression cannot combine {:?} and {:?}", l, r))),
            };
            let result = match op {
                WriteBinOp::Plus => li64.wrapping_add(ri64),
                WriteBinOp::Minus => li64.wrapping_sub(ri64),
                WriteBinOp::Star => li64.wrapping_mul(ri64),
                WriteBinOp::Slash => {
                    if ri64 == 0 {
                        return Ok(Value::Null);
                    }
                    li64.wrapping_div(ri64)
                }
                WriteBinOp::Percent => {
                    if ri64 == 0 {
                        return Ok(Value::Null);
                    }
                    li64.wrapping_rem(ri64)
                }
            };
            Ok(Value::I64(result))
        }
    }
}

fn eval_float_bin(op: WriteBinOp, l: f64, r: f64) -> Result<f64> {
    Ok(match op {
        WriteBinOp::Plus => l + r,
        WriteBinOp::Minus => l - r,
        WriteBinOp::Star => l * r,
        WriteBinOp::Slash => l / r,
        WriteBinOp::Percent => l % r,
    })
}

/// Insert every row of one statement and publish them together.
///
/// Row boundaries come from the caller (`rows`), never from the column count.
/// Staging gives the statement its own atomicity: coercion, primary-key
/// presence and uniqueness are all checked before the first row reaches the
/// record map, so a failing row leaves the table exactly as it was. A repeated
/// primary key inside the same statement is rejected as well, because the
/// record map would otherwise silently keep only the last row.
fn insert_rows(
    table: &str,
    rows: &[Vec<WriteField>],
    contract: &ResolvedContract,
    declared: &[TableUniqueConstraint],
    records: &mut BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
) -> Result<Vec<QueryRow>> {
    let (entry, storage) = resolve_table(contract, table)?;
    let pk_field = catalog::primary_field_name(entry)?;

    let existing_rows =
        if constraints::has_rules(declared, &storage) { store::load_table(records, &storage)? } else { Vec::new() };

    let mut staged: Vec<QueryRow> = Vec::with_capacity(rows.len());
    let mut staged_pks: Vec<String> = Vec::with_capacity(rows.len());
    for fields in rows {
        let mut row = QueryRow::new();
        for field in fields {
            let slot = catalog::field_on_table(entry, &field.name)?;
            let value = resolve_value(&field.value, bound_values)?;
            row.insert(field.name.clone(), coerce::value_to_scalar(value.clone(), &slot.canonical_type)?);
        }
        let pk = row
            .get(pk_field)
            .ok_or_else(|| write_error(format!("insert into `{table}` is missing primary key field `{pk_field}`")))?;
        let pk_text = constraints::value_text(pk)?;
        if staged_pks.contains(&pk_text) {
            return Err(Error::UniqueViolation {
                table: storage.clone(),
                constraint: format!("{table}_pkey"),
                columns: vec![pk_field.to_string()],
            });
        }
        constraints::check_against_rows(&staged, declared, &storage, pk_field, &pk_text, &row)?;
        constraints::check_against_rows(&existing_rows, declared, &storage, pk_field, &pk_text, &row)?;
        staged_pks.push(pk_text);
        staged.push(row);
    }

    // Encode into a pending map first so an encoding failure also aborts before
    // any row is published.
    let mut pending: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (pk_text, row) in staged_pks.iter().zip(&staged) {
        store::upsert_row(&mut pending, &storage, pk_text, row)?;
    }
    records.extend(pending);
    Ok(staged)
}

/// Insert the rows a read pipeline produced (`INSERT INTO … SELECT …`).
///
/// The pipeline runs against the statement's starting snapshot before the first
/// row is staged, so a self-referencing source (`INSERT INTO t SELECT … FROM t`)
/// reads the pre-insert rows rather than its own output. Rows are then mapped to
/// target columns by position and handed to [`insert_rows`], which is the same
/// staging path `VALUES` uses: coercion, primary-key presence and uniqueness are
/// all checked before anything is published.
fn insert_from_query(
    table: &str,
    columns: &[String],
    source: &[QueryOp],
    outputs: Option<&[String]>,
    contract: &ResolvedContract,
    declared: &[TableUniqueConstraint],
    records: &mut BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
) -> Result<Vec<QueryRow>> {
    let source_rows = exec::execute_ops(source, Some(contract), records, bound_values)?;

    let (entry, _storage) = resolve_table(contract, table)?;
    let target_columns: Vec<String> = if columns.is_empty() {
        entry.fields.iter().map(|field| field.canonical_name.clone()).collect()
    }
    else {
        columns.to_vec()
    };
    let output_fields: Vec<String> = match outputs {
        Some(names) => names.to_vec(),
        None => star_output_fields(contract, source)?,
    };
    if target_columns.len() != output_fields.len() {
        return Err(write_error(format!(
            "INSERT ... SELECT column count mismatch: {} target columns, {} source columns",
            target_columns.len(),
            output_fields.len()
        )));
    }

    let rows: Vec<Vec<WriteField>> = source_rows
        .iter()
        .map(|row| {
            target_columns
                .iter()
                .zip(&output_fields)
                .map(|(target, output)| WriteField {
                    name: target.clone(),
                    value: BoundValue::Literal(row.get(output).cloned().unwrap_or(Value::Null)),
                })
                .collect()
        })
        .collect();
    insert_rows(table, &rows, contract, declared, records, bound_values)
}

/// Columns a `SELECT *` source produces, in catalog declaration order.
///
/// The read pipeline keeps `*` as "whatever the scan loaded", so only a single
/// scanned table has an unambiguous column order.
fn star_output_fields(contract: &ResolvedContract, source: &[QueryOp]) -> Result<Vec<String>> {
    let mut scanned = source.iter().filter_map(|op| match op {
        QueryOp::Scan { table } => Some(table),
        _ => None,
    });
    let Some(table) = scanned.next()
    else {
        return Err(write_error("INSERT ... SELECT `*` requires a source table scan"));
    };
    if scanned.next().is_some() {
        return Err(write_error(format!(
            "INSERT ... SELECT `*` requires a single source table: `{table}` is joined with another table, list the source columns instead"
        )));
    }
    let entry = catalog::table_type_by_name(contract, table)?;
    Ok(entry.fields.iter().map(|field| field.canonical_name.clone()).collect())
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
                let value = eval_write_expr(&assignment.value, &row, bound_values)?;
                row.insert(assignment.field.clone(), coerce::value_to_scalar(value, &slot.canonical_type)?);
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
