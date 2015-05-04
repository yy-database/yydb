//! Execute backend-neutral write operators against stored table rows.
//!
//! Every write frontend lowers into [`WriteOp`]; this module resolves those
//! ops against the catalog, enforces uniqueness rules and mutates the record
//! map. Like the read pipeline it never sees VOS or SQL syntax.

use std::collections::BTreeMap;

use vos::types::{ResolvedContract, ResolvedTypeContract};
use yydb_types::{Error, Result, TableUniqueConstraint, UniqueConstraint, Value};

use super::{
    catalog, coerce, constraints, exec,
    ops::{
        BoundValue, ConflictAction, ConflictClause, Expr, Pred, ProjectExpr, ProjectField, QueryOp, QueryRow,
        ReturningProjection, WriteAssignment, WriteBinOp, WriteExpr, WriteField, WriteOp,
    },
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
    /// Rows handed back by the write statement: the projection of its
    /// `RETURNING` clause when present, otherwise the written rows themselves.
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
            WriteOp::Insert { table, rows, conflict, returning } => {
                let inserted = insert_rows(
                    table,
                    rows,
                    conflict.as_ref(),
                    returning.as_ref(),
                    contract,
                    declared,
                    records,
                    bound_values,
                )?;
                outcome.affected += inserted.len();
                outcome.rows.extend(inserted);
            }
            WriteOp::InsertFromQuery { table, columns, source, outputs, conflict, returning } => {
                let inserted = insert_from_query(
                    table,
                    columns,
                    source,
                    outputs.as_deref(),
                    conflict.as_ref(),
                    returning.as_ref(),
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

/// A row the statement can see: either already stored or staged by an earlier
/// row of the same statement.
struct WorkingRow {
    /// Record-map key the row is (or will be) published under.
    key: String,
    /// Column values of the row.
    row: QueryRow,
}

/// A resolved `ON CONFLICT` target: the uniqueness rule backing the named
/// columns, or `None` when the target is the primary key.
struct ConflictTarget<'a> {
    constraint: Option<&'a UniqueConstraint>,
}

/// Resolve the conflict target columns against the catalog and the rules this
/// write path enforces.
///
/// A target that neither covers the primary key nor matches a declared unique
/// constraint is rejected, because the statement would otherwise react to
/// conflicts on columns it never named.
fn conflict_target<'a>(
    entry: &ResolvedTypeContract,
    declared: &'a [TableUniqueConstraint],
    storage: &str,
    pk_field: &str,
    columns: &[String],
    table: &str,
) -> Result<ConflictTarget<'a>> {
    if columns.is_empty() {
        return Err(write_error(format!(
            "ON CONFLICT on `{table}` needs an explicit target column list naming the primary key or unique columns it reacts to"
        )));
    }
    let mut resolved: Vec<String> = Vec::with_capacity(columns.len());
    for column in columns {
        let slot = catalog::field_on_table(entry, column)?;
        if resolved.contains(&slot.canonical_name) {
            return Err(write_error(format!("ON CONFLICT target column `{column}` appears more than once")));
        }
        resolved.push(slot.canonical_name.clone());
    }
    if resolved.len() == 1 && resolved[0] == pk_field {
        return Ok(ConflictTarget { constraint: None });
    }
    let mut wanted = resolved.clone();
    wanted.sort();
    for candidate in declared.iter().filter(|candidate| candidate.table() == storage) {
        let constraint = candidate.constraint();
        let mut covered = constraint.columns().to_vec();
        covered.sort();
        if covered == wanted {
            return Ok(ConflictTarget { constraint: Some(constraint) });
        }
    }
    Err(write_error(format!(
        "ON CONFLICT target ({}) on `{table}` is not backed by a unique constraint: name the primary key or the exact columns of a unique constraint",
        resolved.join(", ")
    )))
}

/// Index of the first visible row `row` conflicts with on `target`.
fn conflicting_row(
    working: &[WorkingRow],
    target: &ConflictTarget<'_>,
    pk_field: &str,
    pk_text: &str,
    row: &QueryRow,
) -> Result<Option<usize>> {
    for (index, candidate) in working.iter().enumerate() {
        let collides = match target.constraint {
            Some(constraint) => constraints::keys_conflict(constraint, &candidate.row, row),
            None => constraints::value_text(candidate.row.get(pk_field).unwrap_or(&Value::Null))? == pk_text,
        };
        if collides {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

/// Insert every row of one statement and publish them together.
///
/// Row boundaries come from the caller (`rows`), never from the column count.
/// Staging gives the statement its own atomicity: coercion, primary-key
/// presence and uniqueness are all checked before the first row reaches the
/// record map, so a failing row leaves the table exactly as it was. A repeated
/// primary key inside the same statement is rejected as well, because the
/// record map would otherwise silently keep only the last row.
///
/// A row that conflicts on the `ON CONFLICT` target follows the clause's
/// action instead of failing. Rows already stored and rows staged by earlier
/// rows of the same statement both count as visible for that check, so the
/// clause sees the same table state a later statement would.
///
/// The returned rows are the `RETURNING` projection of every row this statement
/// wrote (inserted, or updated by `DO UPDATE`), or the written rows themselves
/// when the statement has no `RETURNING`. Projection happens before publication,
/// so an unknown column in the clause fails without publishing anything.
fn insert_rows(
    table: &str,
    rows: &[Vec<WriteField>],
    conflict: Option<&ConflictClause>,
    returning: Option<&ReturningProjection>,
    contract: &ResolvedContract,
    declared: &[TableUniqueConstraint],
    records: &mut BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
) -> Result<Vec<QueryRow>> {
    let (entry, storage) = resolve_table(contract, table)?;
    let pk_field = catalog::primary_field_name(entry)?.to_string();
    let target = conflict
        .map(|clause| conflict_target(entry, declared, &storage, &pk_field, &clause.columns, table))
        .transpose()?;
    let returning = match returning {
        None | Some(ReturningProjection::Star) => None,
        Some(ReturningProjection::Fields(fields)) => {
            for field in fields {
                validate_returning(entry, table, &field.expr)?;
            }
            Some(fields.as_slice())
        }
    };

    // Uniqueness checks and conflict detection both compare against the current
    // table contents, so the rows are loaded whenever either is in play.
    let mut working: Vec<WorkingRow> = if target.is_some() || constraints::has_rules(declared, &storage) {
        store::load_table_entries(records, &storage)?
            .into_iter()
            .map(|(key, row)| WorkingRow { key, row })
            .collect()
    }
    else {
        Vec::new()
    };

    let mut pending: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut staged_pks: Vec<String> = Vec::with_capacity(rows.len());
    let mut written: Vec<QueryRow> = Vec::with_capacity(rows.len());

    for fields in rows {
        let mut row = QueryRow::new();
        for field in fields {
            let slot = catalog::field_on_table(entry, &field.name)?;
            let value = resolve_value(&field.value, bound_values)?;
            row.insert(field.name.clone(), coerce::value_to_scalar(value.clone(), &slot.canonical_type)?);
        }
        let pk = row
            .get(&pk_field)
            .ok_or_else(|| write_error(format!("insert into `{table}` is missing primary key field `{pk_field}`")))?;
        let pk_text = constraints::value_text(pk)?;

        if let (Some(target), Some(clause)) = (&target, conflict)
            && let Some(index) = conflicting_row(&working, target, &pk_field, &pk_text, &row)?
        {
            match &clause.action {
                ConflictAction::DoNothing => continue,
                ConflictAction::DoUpdate { assignments, predicate } => {
                    let Some(updated) = conflict_update_row(
                        table,
                        entry,
                        &pk_field,
                        &working[index].row,
                        assignments,
                        predicate.as_ref(),
                        &working,
                        declared,
                        &storage,
                        records,
                        bound_values,
                    )?
                    else {
                        continue;
                    };
                    // The updated row keeps the conflicting row's primary key,
                    // which is not the incoming row's key when the target is a
                    // unique column rather than the primary key.
                    let matched_pk = updated
                        .get(&pk_field)
                        .ok_or_else(|| write_error(format!("updated `{table}` row lost primary key field `{pk_field}`")))?;
                    let matched_pk_text = constraints::value_text(matched_pk)?;
                    store::upsert_row(&mut pending, &storage, &matched_pk_text, &updated)?;
                    working[index].row = updated.clone();
                    written.push(project_returned(returning, &updated, table, contract, records, bound_values)?);
                    continue;
                }
            }
        }

        if staged_pks.contains(&pk_text) {
            return Err(Error::UniqueViolation {
                table: storage.clone(),
                constraint: format!("{table}_pkey"),
                columns: vec![pk_field.clone()],
            });
        }
        let known: Vec<QueryRow> = working.iter().map(|candidate| candidate.row.clone()).collect();
        constraints::check_against_rows(&known, declared, &storage, &pk_field, &pk_text, &row)?;

        let key = store::row_key(&storage, &pk_text);
        store::upsert_row(&mut pending, &storage, &pk_text, &row)?;
        match working.iter_mut().find(|candidate| candidate.key == key) {
            Some(existing) => existing.row = row.clone(),
            None => working.push(WorkingRow { key, row: row.clone() }),
        }
        staged_pks.push(pk_text);
        written.push(project_returned(returning, &row, table, contract, records, bound_values)?);
    }

    // The staged map is applied in one move so an encoding failure aborts
    // before any row is published.
    records.extend(pending);
    Ok(written)
}

/// Project one written row through the statement's `RETURNING` fields.
///
/// `None` means the statement either has no clause or wrote `RETURNING *`, both
/// of which hand back the written row unchanged.
fn project_returned(
    fields: Option<&[ProjectField]>,
    row: &QueryRow,
    table: &str,
    contract: &ResolvedContract,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
) -> Result<QueryRow> {
    match fields {
        Some(fields) => exec::project_row(fields, row, table, Some(contract), records, bound_values),
        None => Ok(row.clone()),
    }
}

/// Reject a `RETURNING` item that names a column the target table does not have.
///
/// Reads resolve unknown columns to `NULL`, but a write clause that silently
/// returns `NULL` would look like the column exists, so every referenced column
/// is resolved against the catalog before the statement publishes anything.
fn validate_returning(entry: &ResolvedTypeContract, table: &str, expr: &ProjectExpr) -> Result<()> {
    match expr {
        ProjectExpr::Scalar { path } => validate_returning_column(entry, table, path),
        ProjectExpr::Computed(inner) => validate_returning_expr(entry, table, inner),
        ProjectExpr::Nested { .. } => {
            Err(write_error(format!("RETURNING on `{table}` does not support nested projections")))
        }
    }
}

fn validate_returning_expr(entry: &ResolvedTypeContract, table: &str, expr: &Expr) -> Result<()> {
    match expr {
        Expr::Column { path } => validate_returning_column(entry, table, path),
        Expr::Bound(_) => Ok(()),
        Expr::Case { branches, else_branch } => {
            for (when, then) in branches {
                validate_returning_expr(entry, table, when)?;
                validate_returning_expr(entry, table, then)?;
            }
            match else_branch {
                Some(else_branch) => validate_returning_expr(entry, table, else_branch),
                None => Ok(()),
            }
        }
        Expr::Cast { expr, .. } => validate_returning_expr(entry, table, expr),
        Expr::Binary { left, right, .. } => {
            validate_returning_expr(entry, table, left)?;
            validate_returning_expr(entry, table, right)
        }
        Expr::Not(inner) => validate_returning_expr(entry, table, inner),
    }
}

fn validate_returning_column(entry: &ResolvedTypeContract, table: &str, path: &[String]) -> Result<()> {
    let Some(first) = path.first()
    else {
        return Err(write_error(format!("RETURNING on `{table}` has an empty column path")));
    };
    catalog::field_on_table(entry, first)?;
    if path.len() > 1 {
        return Err(write_error(format!(
            "RETURNING column `{}` must name a column of `{table}` without a table qualifier",
            path.join(".")
        )));
    }
    Ok(())
}

/// Apply the `DO UPDATE` assignments to the conflicting stored row.
///
/// Both the optional predicate and the assignment expressions read the
/// *conflicting* row — the row the uniqueness rule matched, which is not the
/// incoming row when the target is a unique column rather than the primary key.
/// `None` means the predicate rejected the update: the incoming row is dropped
/// and the conflicting row stays as it was. The updated row still has to pass
/// the table's uniqueness rules before the caller publishes it.
fn conflict_update_row(
    table: &str,
    entry: &ResolvedTypeContract,
    pk_field: &str,
    matched: &QueryRow,
    assignments: &[WriteAssignment],
    predicate: Option<&Pred>,
    working: &[WorkingRow],
    declared: &[TableUniqueConstraint],
    storage: &str,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
) -> Result<Option<QueryRow>> {
    if let Some(predicate) = predicate {
        let predicate = exec::resolve_subquery_predicates(predicate, None, records, bound_values)?;
        if !exec::eval_pred(&predicate, matched, table, None, records, bound_values) {
            return Ok(None);
        }
    }

    let mut row = matched.clone();
    for assignment in assignments {
        let slot = catalog::field_on_table(entry, &assignment.field)?;
        if assignment.field == pk_field {
            return Err(write_error(format!("primary key field `{pk_field}` cannot be updated")));
        }
        let value = eval_write_expr(&assignment.value, matched, bound_values)?;
        row.insert(assignment.field.clone(), coerce::value_to_scalar(value, &slot.canonical_type)?);
    }

    let matched_pk = matched
        .get(pk_field)
        .ok_or_else(|| write_error(format!("conflicting `{table}` row has no primary key field `{pk_field}`")))?;
    let matched_pk_text = constraints::value_text(matched_pk)?;
    let known: Vec<QueryRow> = working.iter().map(|candidate| candidate.row.clone()).collect();
    constraints::check_against_rows(&known, declared, storage, pk_field, &matched_pk_text, &row)?;
    Ok(Some(row))
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
    conflict: Option<&ConflictClause>,
    returning: Option<&ReturningProjection>,
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
    insert_rows(table, &rows, conflict, returning, contract, declared, records, bound_values)
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
    let predicate = predicate
        .map(|predicate| exec::resolve_subquery_predicates(predicate, None, records, bound_values))
        .transpose()?;
    let predicate = predicate.as_ref();

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
    let predicate = predicate
        .map(|predicate| exec::resolve_subquery_predicates(predicate, None, records, bound_values))
        .transpose()?;
    let predicate = predicate.as_ref();
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
