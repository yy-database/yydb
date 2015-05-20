//! Execute Phase 1 query ops against stored table rows.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use vos::types::ResolvedContract;
use yydb_types::{Error, Result, Value};

use super::{
    catalog::table_row_key_name,
    lower,
    ops::{
        AggregateField, BinaryOp, BoundValue, CmpOp, Expr, JoinKind, OUTER_ROW_MARKER, Pred, ProjectExpr, ProjectField,
        QueryOp, QueryRow, ScalarFunction, SetOpKind, WindowField, WindowFrameBound, WindowFunction,
    },
    resolve, store,
};

/// Execute a VOS source directly (convenience wrapper). Parses + lowers + runs.
pub fn execute(
    source: &str,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<QueryRow>> {
    let program = vos::parse_program(source).map_err(|diagnostics| Error::Schema { message: diagnostics })?;
    let ops = lower::lower_program(&program, &[])?;
    execute_ops(&ops, contract, records, &[])
}

/// Row of the statement that encloses a correlated subquery, plus the table it
/// was read from. Correlated column paths carry [`OUTER_ROW_MARKER`] and resolve
/// against this row.
pub(crate) struct OuterRow<'a> {
    pub(crate) row: &'a QueryRow,
    pub(crate) table: &'a str,
}

/// Execute a pre-lowered op list. `bound_values` supplies parameters for
/// `BoundValue::Slot { index }` entries; pass `&[]` when all values are
/// `BoundValue::Literal`.
pub fn execute_ops(
    ops: &[QueryOp],
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
) -> Result<Vec<QueryRow>> {
    execute_ops_in(ops, contract, records, bound_values, None)
}

/// Execute a pipeline on behalf of `outer`, the row of the statement that
/// encloses it.
///
/// `outer` is what a correlated subquery reads: lowering marked those paths, and
/// execution resolves them against this row. A pipeline that is not nested inside
/// another statement passes `None`.
fn execute_ops_in(
    ops: &[QueryOp],
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    outer: Option<&OuterRow<'_>>,
) -> Result<Vec<QueryRow>> {
    let mut rows: Vec<QueryRow> = Vec::new();
    let mut current_table: Option<String> = None;
    for op in ops {
        match op {
            QueryOp::Scan { table } => {
                let storage_table = match contract {
                    Some(contract) => table_row_key_name(contract, table)?,
                    None => table.clone(),
                };
                current_table = Some(table.clone());
                rows = store::load_table(records, &storage_table)?;
            }
            QueryOp::Qualify { prefix } => {
                rows = rows
                    .into_iter()
                    .map(|row| row.into_iter().map(|(key, value)| (format!("{prefix}.{key}"), value)).collect())
                    .collect();
            }
            QueryOp::Filter { predicate } => {
                let table = current_table
                    .as_deref()
                    .ok_or_else(|| Error::Schema { message: "filter without a preceding table scan".into() })?;
                if predicate_has_correlated_subquery(predicate) {
                    // A correlated subquery reads the row under test, so it is
                    // re-run for every row instead of once for the pipeline.
                    let mut kept: Vec<QueryRow> = Vec::with_capacity(rows.len());
                    for row in std::mem::take(&mut rows) {
                        if eval_pred_for_row(predicate, &row, table, contract, records, bound_values, outer)? {
                            kept.push(row);
                        }
                    }
                    rows = kept;
                }
                else {
                    let predicate = resolve_subquery_predicates_in(predicate, contract, records, bound_values, None)?;
                    rows.retain(|row| eval_pred_in(&predicate, row, table, contract, records, bound_values, outer));
                }
            }
            QueryOp::Join { kind, right, right_alias, on } => {
                if current_table.is_none() {
                    return Err(Error::Schema { message: "join without a preceding table scan".into() });
                }
                let storage_right = match contract {
                    Some(contract) => table_row_key_name(contract, right)?,
                    None => right.clone(),
                };
                let right_rows = store::load_table(records, &storage_right)?;
                let prefix = right_alias.clone().unwrap_or_else(|| right.clone());
                rows = execute_join(*kind, &rows, &right_rows, &prefix, on, contract, records, bound_values, outer)?;
            }
            QueryOp::JoinSubquery { kind, right, right_alias, on } => {
                if current_table.is_none() {
                    return Err(Error::Schema { message: "join without a preceding table scan".into() });
                }
                let right_rows = execute_ops(right, contract, records, bound_values)?;
                rows = execute_join(*kind, &rows, &right_rows, right_alias, on, contract, records, bound_values, outer)?;
            }
            QueryOp::SetOperation { kind, left, right, columns, right_columns } => {
                rows = execute_set_operation(
                    *kind,
                    left,
                    right,
                    columns.as_deref(),
                    right_columns.as_deref(),
                    contract,
                    records,
                    bound_values,
                    outer,
                )?;
            }
            QueryOp::Aggregate { group_keys, aggregates } => {
                rows = execute_aggregate(&rows, group_keys, aggregates, contract, records, bound_values, outer)?;
            }
            QueryOp::Window { fields } => {
                rows = execute_window(&rows, fields, contract, records, bound_values, outer)?;
            }
            QueryOp::Project { fields } => {
                let table = current_table
                    .as_deref()
                    .ok_or_else(|| Error::Schema { message: "project without a preceding table scan".into() })?;
                // An uncorrelated scalar subquery does not read the row being
                // projected, so it is resolved once for the whole pipeline. A
                // correlated one stays in place and is evaluated per row.
                let fields = resolve_projection_subqueries(fields, contract, records, bound_values)?;
                rows = rows
                    .into_iter()
                    .map(|row| project_row_in(&fields, &row, table, contract, records, bound_values, outer))
                    .collect::<Result<Vec<_>>>()?;
            }
            QueryOp::Sort { keys } => {
                rows.sort_by(|left, right| {
                    for key in keys {
                        let left_value = left.get(&key.field).unwrap_or(&Value::Null);
                        let right_value = right.get(&key.field).unwrap_or(&Value::Null);
                        let ord = compare_values(left_value, right_value);
                        let ord = if key.ascending { ord } else { ord.reverse() };
                        if ord != std::cmp::Ordering::Equal {
                            return ord;
                        }
                    }
                    std::cmp::Ordering::Equal
                });
            }
            QueryOp::Skip { count } => {
                let skip = match resolve_bound_value(count, bound_values)? {
                    Value::I64(n) => (n as usize).min(rows.len()),
                    _ => return Err(query_runtime_error("Skip/Take count must be an integer")),
                };
                rows = rows.into_iter().skip(skip).collect();
            }
            QueryOp::Take { count } => {
                let take = match resolve_bound_value(count, bound_values)? {
                    Value::I64(n) => n as usize,
                    _ => return Err(query_runtime_error("Skip/Take count must be an integer")),
                };
                rows.truncate(take);
            }
            QueryOp::Collect => {}
        }
    }
    Ok(rows)
}

fn query_runtime_error(message: impl Into<String>) -> Error {
    Error::Schema { message: message.into() }
}

/// Resolve a BoundValue either to the baked-in literal or to the bound
/// parameter at `index`. Returns an error for Slot references that exceed
/// the caller-supplied slice (bind-time contract violation).
fn resolve_bound_value(bv: &BoundValue, bound_values: &[Value]) -> Result<Value> {
    match bv {
        BoundValue::Literal(v) => Ok(v.clone()),
        BoundValue::Slot { index, .. } => bound_values
            .get(*index as usize)
            .cloned()
            .ok_or_else(|| Error::Schema { message: format!("bound parameter index {} out of range", index) }),
    }
}

pub fn project_row(
    fields: &[ProjectField],
    row: &QueryRow,
    table: &str,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
) -> Result<QueryRow> {
    project_row_in(fields, row, table, contract, records, bound_values, None)
}

fn project_row_in(
    fields: &[ProjectField],
    row: &QueryRow,
    table: &str,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    outer: Option<&OuterRow<'_>>,
) -> Result<QueryRow> {
    let mut out = QueryRow::new();
    for field in fields {
        match &field.expr {
            ProjectExpr::Scalar { path } => {
                let value = read_path(path, row, table, contract, records, outer);
                out.insert(field.name.clone(), value);
            }
            ProjectExpr::Nested { path, fields: nested } => {
                let nested_value = match contract {
                    Some(contract) => {
                        let resolved = resolve::resolve_row_at_path(path, row, table, contract, records);
                        match resolved {
                            Ok((nested_row, nested_table)) => {
                                let nested_projected = project_row_in(
                                    nested,
                                    &nested_row,
                                    &nested_table,
                                    Some(contract),
                                    records,
                                    bound_values,
                                    outer,
                                )?;
                                Value::Row(nested_projected)
                            }
                            Err(_) => Value::Null,
                        }
                    }
                    None => Value::Null,
                };
                out.insert(field.name.clone(), nested_value);
            }
            ProjectExpr::Computed(inner) => {
                let value = eval_expr(inner, row, table, contract, records, bound_values, outer)?;
                out.insert(field.name.clone(), value);
            }
        }
    }
    Ok(out)
}

/// Resolve every `[NOT] EXISTS (subquery)` and `[NOT] IN (subquery)` node of a
/// predicate.
///
/// An `EXISTS` subquery collapses to a constant. An `IN` subquery collapses to
/// the set of values it produced, which the row under test is then compared
/// against. This variant expects uncorrelated nodes only; a correlated one needs
/// the row of the statement that encloses it and is resolved by
/// [`eval_pred_for_row`] instead.
pub(crate) fn resolve_subquery_predicates(
    predicate: &Pred,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
) -> Result<Pred> {
    resolve_subquery_predicates_in(predicate, contract, records, bound_values, None)
}

/// Resolve every subquery node of a predicate against `outer`, the row of the
/// enclosing statement.
///
/// A correlated node is executed once here with that row bound; an uncorrelated
/// one is executed once and reused.
fn resolve_subquery_predicates_in(
    predicate: &Pred,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    outer: Option<&OuterRow<'_>>,
) -> Result<Pred> {
    let resolve = |inner: &Pred| resolve_subquery_predicates_in(inner, contract, records, bound_values, outer);
    Ok(match predicate {
        Pred::Exists { ops, negated, correlated } => {
            let found = !execute_subquery(ops, contract, records, bound_values, *correlated, outer)?.is_empty();
            if found == *negated { Pred::False } else { Pred::True }
        }
        Pred::InSubquery { path, ops, negated, correlated } => {
            let rows = execute_subquery(ops, contract, records, bound_values, *correlated, outer)?;
            let mut values = Vec::with_capacity(rows.len());
            for row in &rows {
                if row.len() != 1 {
                    return Err(query_runtime_error(format!(
                        "`IN (SELECT ...)` requires a single-column subquery, but its rows carry {} columns",
                        row.len()
                    )));
                }
                values.push(row.values().next().cloned().unwrap_or(Value::Null));
            }
            Pred::InSet { path: path.clone(), values, negated: *negated }
        }
        Pred::FieldCmpSubquery { path, op, ops, correlated } => Pred::FieldCmpScalar {
            path: path.clone(),
            op: *op,
            value: scalar_subquery_value(ops, contract, records, bound_values, *correlated, outer)?,
        },
        Pred::And(left, right) => Pred::And(Box::new(resolve(left)?), Box::new(resolve(right)?)),
        Pred::Or(left, right) => Pred::Or(Box::new(resolve(left)?), Box::new(resolve(right)?)),
        Pred::Not(inner) => Pred::Not(Box::new(resolve(inner)?)),
        other => other.clone(),
    })
}

/// Whether a predicate carries a subquery that reads the row under test, which
/// forces a per-row run instead of one run per pipeline.
fn predicate_has_correlated_subquery(predicate: &Pred) -> bool {
    match predicate {
        Pred::Exists { correlated, .. } | Pred::InSubquery { correlated, .. } | Pred::FieldCmpSubquery { correlated, .. } => {
            *correlated
        }
        Pred::And(left, right) | Pred::Or(left, right) => {
            predicate_has_correlated_subquery(left) || predicate_has_correlated_subquery(right)
        }
        Pred::Not(inner) => predicate_has_correlated_subquery(inner),
        _ => false,
    }
}

/// Evaluate one predicate against one row.
///
/// A correlated subquery is re-run with this row as the enclosing row, which is
/// what per-row re-execution means. `outer` is the row of the statement that
/// encloses this pipeline, if any, and only resolves column paths that the
/// pipeline's own statement marked as outer references.
fn eval_pred_for_row(
    predicate: &Pred,
    row: &QueryRow,
    table: &str,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    outer: Option<&OuterRow<'_>>,
) -> Result<bool> {
    let row_outer = OuterRow { row, table };
    let predicate = if predicate_has_correlated_subquery(predicate) {
        resolve_subquery_predicates_in(predicate, contract, records, bound_values, Some(&row_outer))?
    }
    else {
        resolve_subquery_predicates_in(predicate, contract, records, bound_values, None)?
    };
    Ok(eval_pred_in(&predicate, row, table, contract, records, bound_values, outer))
}

/// Run one subquery pipeline.
///
/// A correlated one is executed against `outer`, the row of the statement that
/// encloses it. An uncorrelated one runs once on its own.
fn execute_subquery(
    ops: &[QueryOp],
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    correlated: bool,
    outer: Option<&OuterRow<'_>>,
) -> Result<Vec<QueryRow>> {
    if !correlated {
        return execute_ops(ops, contract, records, bound_values);
    }
    let Some(outer) = outer
    else {
        return Err(query_runtime_error("a correlated subquery reached execution without the row of its enclosing statement"));
    };
    execute_ops_in(ops, contract, records, bound_values, Some(outer))
}

pub(crate) fn eval_pred(
    pred: &Pred,
    row: &QueryRow,
    table: &str,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
) -> bool {
    eval_pred_in(pred, row, table, contract, records, bound_values, None)
}

fn eval_pred_in(
    pred: &Pred,
    row: &QueryRow,
    table: &str,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    outer: Option<&OuterRow<'_>>,
) -> bool {
    match pred {
        Pred::True => true,
        Pred::False => false,
        Pred::FieldBool { path, value } => {
            let left = read_path(path, row, table, contract, records, outer);
            let right = match resolve_bound_value(value, bound_values) {
                Ok(v) => v,
                Err(_) => return false, // unresolved slot → predicate fails safe
            };
            matches!((&left, &right), (Value::Bool(a), Value::Bool(b)) if a == b)
        }
        Pred::FieldCmp { path, op, value } => {
            let left = read_path(path, row, table, contract, records, outer);
            let right = match resolve_bound_value(value, bound_values) {
                Ok(v) => v,
                Err(_) => return false,
            };
            cmp_values(&left, *op, &right)
        }
        Pred::FieldCmpField { left_path, op, right_path } => {
            let left = read_path(left_path, row, table, contract, records, outer);
            let right = read_path(right_path, row, table, contract, records, outer);
            cmp_values(&left, *op, &right)
        }
        Pred::And(left, right) => {
            eval_pred_in(left, row, table, contract, records, bound_values, outer)
                && eval_pred_in(right, row, table, contract, records, bound_values, outer)
        }
        Pred::Or(left, right) => {
            eval_pred_in(left, row, table, contract, records, bound_values, outer)
                || eval_pred_in(right, row, table, contract, records, bound_values, outer)
        }
        Pred::Not(inner) => !eval_pred_in(inner, row, table, contract, records, bound_values, outer),
        Pred::InSet { path, values, negated } => {
            let left = read_path(path, row, table, contract, records, outer);
            // `WHERE` keeps a row only on TRUE, so an unknown test drops it —
            // which is also what `NOT IN` does over a set that contains NULL.
            match in_set_truth(&left, values) {
                Some(true) => !*negated,
                Some(false) => *negated,
                None => false,
            }
        }
        // A scalar subquery is resolved before any row is tested. A NULL on
        // either side makes the comparison unknown, so no row may pass.
        Pred::FieldCmpScalar { path, op, value } => {
            let left = read_path(path, row, table, contract, records, outer);
            if matches!(left, Value::Null) || matches!(value, Value::Null) {
                return false;
            }
            cmp_values(&left, *op, value)
        }
        // Subqueries are resolved before any row is tested. Reaching one here
        // means the caller skipped that step, so no row may pass.
        Pred::Exists { .. } | Pred::InSubquery { .. } | Pred::FieldCmpSubquery { .. } => false,
    }
}

/// Scalar value a `(SELECT ...)` value subquery yields.
///
/// Zero rows yield `NULL`. Exactly one row must carry exactly one column.
/// More than one row is an error: silently keeping the first row would hide a
/// subquery that is not actually scalar, and a wider row is rejected rather
/// than comparing only its first column.
fn scalar_subquery_value(
    ops: &[QueryOp],
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    correlated: bool,
    outer: Option<&OuterRow<'_>>,
) -> Result<Value> {
    let rows = execute_subquery(ops, contract, records, bound_values, correlated, outer)?;
    let mut rows = rows.into_iter();
    let Some(row) = rows.next()
    else {
        return Ok(Value::Null);
    };
    if rows.next().is_some() {
        return Err(query_runtime_error("a scalar subquery must return at most one row: use an aggregate or add a filter"));
    }
    if row.len() != 1 {
        return Err(query_runtime_error(format!(
            "a scalar subquery must produce a single column, but its row carries {} columns",
            row.len()
        )));
    }
    Ok(row.into_values().next().unwrap_or(Value::Null))
}

/// Three-valued result of `value [NOT] IN (<values>)`.
///
/// - An empty set is always decided: `IN` is the disjunction of no comparison,
///   which is `FALSE`, so `NOT IN` over an empty set is `TRUE` even when the
///   tested value is `NULL`.
/// - A `NULL` tested value is unknown unless the set is empty.
/// - A match is `TRUE`. Without a match, a `NULL` in the set makes the result
///   unknown, because that comparison could have matched.
fn in_set_truth(value: &Value, values: &[Value]) -> Option<bool> {
    if values.is_empty() {
        return Some(false);
    }
    if matches!(value, Value::Null) {
        return None;
    }
    let mut has_null = false;
    for candidate in values {
        if matches!(candidate, Value::Null) {
            has_null = true;
            continue;
        }
        if cmp_values(value, CmpOp::Eq, candidate) {
            return Some(true);
        }
    }
    if has_null { None } else { Some(false) }
}

fn read_path(
    path: &[String],
    row: &QueryRow,
    table: &str,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    outer: Option<&OuterRow<'_>>,
) -> Value {
    // A path lowering marked as an outer reference is read from the row of the
    // statement that encloses this pipeline, never from the row under test.
    if let Some((first, rest)) = path.split_first()
        && first.as_str() == OUTER_ROW_MARKER
    {
        let Some(outer) = outer
        else {
            return Value::Null;
        };
        return read_path(rest, outer.row, outer.table, contract, records, None);
    }
    // Single-element path — direct bare column lookup, the common case.
    if path.len() == 1 {
        return row.get(&path[0]).cloned().unwrap_or(Value::Null);
    }
    // Multi-element path from a qualified identifier split at the dot (e.g.
    // `Project.owner_id` or `Account.account_id`). Try, in order:
    //   a) the full dotted key — matches right-table columns prefixed by
    //      `merge_rows` (e.g. `Project.owner_id`)
    //   b) the trailing bare column — matches left-table columns that keep
    //      their original name (e.g. `account_id` from `Account.account_id`)
    //   c) nested VOS path resolution via the contract
    let dotted: String = path.join(".");
    if let Some(v) = row.get(&dotted) {
        return v.clone();
    }
    let last = path.last().unwrap();
    if let Some(v) = row.get(last) {
        return v.clone();
    }
    if let Some(contract) = contract {
        return resolve::resolve_field_path(path, row, table, contract, records).unwrap_or(Value::Null);
    }
    Value::Null
}

/// Total order used by comparisons and sorting.
///
/// Number kinds compare with each other: an exact integer pair (`i64`/`u64`)
/// compares as an integer, and any pair that involves a REAL promotes both
/// sides to `f64`. Distinct kind families never compare equal: they order by
/// kind rank (`null` < `bool` < number < text/uuid < everything else), so a
/// comparison such as `text_column = 1.5` never matches instead of pretending
/// the two sides are equal. Two values of the same unmodelled kind also compare
/// equal rather than falling back to an arbitrary order.
fn compare_values(left: &Value, right: &Value) -> std::cmp::Ordering {
    match (left, right) {
        (Value::Null, Value::Null) => std::cmp::Ordering::Equal,
        (Value::Null, _) => std::cmp::Ordering::Less,
        (_, Value::Null) => std::cmp::Ordering::Greater,
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        (Value::I64(a), Value::I64(b)) => a.cmp(b),
        (Value::U64(a), Value::U64(b)) => a.cmp(b),
        (Value::I64(a), Value::U64(b)) => (*a as i128).cmp(&(*b as i128)),
        (Value::U64(a), Value::I64(b)) => (*a as i128).cmp(&(*b as i128)),
        (Value::I64(a), Value::F64(b)) => (*a as f64).total_cmp(b),
        (Value::U64(a), Value::F64(b)) => (*a as f64).total_cmp(b),
        (Value::F64(a), Value::I64(b)) => a.total_cmp(&(*b as f64)),
        (Value::F64(a), Value::U64(b)) => a.total_cmp(&(*b as f64)),
        (Value::F64(a), Value::F64(b)) => a.total_cmp(b),
        (Value::Text(a), Value::Text(b)) => a.cmp(b),
        (Value::Uuid(a), Value::Uuid(b)) => a.cmp(b),
        (Value::Text(a), Value::Uuid(b)) => a.as_str().cmp(&b.to_string()),
        (Value::Uuid(a), Value::Text(b)) => a.to_string().cmp(b),
        (Value::Bytes(a), Value::Bytes(b)) => a.cmp(b),
        _ => value_kind_rank(left).cmp(&value_kind_rank(right)),
    }
}

/// Kind family used to order values that have no common comparison domain.
fn value_kind_rank(value: &Value) -> u8 {
    match value {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::I64(_) | Value::U64(_) | Value::F64(_) => 2,
        Value::Text(_) | Value::Uuid(_) => 3,
        Value::Bytes(_) => 4,
        _ => 5,
    }
}

fn cmp_values(left: &Value, op: CmpOp, right: &Value) -> bool {
    let ord = compare_values(left, right);
    match op {
        CmpOp::Eq => ord == std::cmp::Ordering::Equal,
        CmpOp::Ne => ord != std::cmp::Ordering::Equal,
        CmpOp::Lt => ord == std::cmp::Ordering::Less,
        CmpOp::Le => ord != std::cmp::Ordering::Greater,
        CmpOp::Gt => ord == std::cmp::Ordering::Greater,
        CmpOp::Ge => ord != std::cmp::Ordering::Less,
    }
}

// ─── expression evaluation (CASE / CAST) ──────────────────────────────────

/// Evaluate the backend-neutral [`Expr`] IR against a single row. Returns an
/// `Err` for cast failures and other runtime errors; callers like
/// [`project_row`] are currently catching and falling through to `NULL`.
pub(crate) fn eval_expr(
    expr: &Expr,
    row: &QueryRow,
    table: &str,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    outer: Option<&OuterRow<'_>>,
) -> Result<Value> {
    match expr {
        Expr::Column { path } => Ok(read_path(path, row, table, contract, records, outer)),
        Expr::Bound(bv) => resolve_bound_value(bv, bound_values),
        Expr::Case { branches, else_branch } => {
            eval_case(branches, else_branch, row, table, contract, records, bound_values, outer)
        }
        Expr::Cast { expr: inner, target } => {
            let value = eval_expr(inner, row, table, contract, records, bound_values, outer)?;
            eval_cast(&value, target)
        }
        Expr::Binary { op, left, right } => {
            let l = eval_expr(left, row, table, contract, records, bound_values, outer)?;
            let r = eval_expr(right, row, table, contract, records, bound_values, outer)?;
            eval_binary(*op, &l, &r)
        }
        Expr::Not(inner) => {
            let val = eval_expr(inner, row, table, contract, records, bound_values, outer)?;
            Ok(Value::Bool(!is_truthy(&val)))
        }
        Expr::Function { function, args } => eval_function(*function, args, row, table, contract, records, bound_values, outer),
        // A correlated value subquery reads the row being projected, so that row
        // becomes the enclosing row of its per-row run.
        Expr::Subquery { ops, correlated } => {
            let enclosing = OuterRow { row, table };
            let outer = if *correlated { Some(&enclosing) } else { None };
            scalar_subquery_value(ops, contract, records, bound_values, *correlated, outer)
        }
    }
}

/// Resolve every scalar subquery of a projection before any row is projected.
///
/// The subqueries are uncorrelated, so running them once per pipeline keeps the
/// per-row evaluation free of storage access and surfaces their errors before
/// the first row is emitted.
fn resolve_projection_subqueries(
    fields: &[ProjectField],
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
) -> Result<Vec<ProjectField>> {
    fields
        .iter()
        .map(|field| {
            let expr = match &field.expr {
                ProjectExpr::Computed(inner) => {
                    ProjectExpr::Computed(Box::new(resolve_scalar_subqueries(inner, contract, records, bound_values)?))
                }
                other => other.clone(),
            };
            Ok(ProjectField { name: field.name.clone(), expr })
        })
        .collect()
}

/// Replace every uncorrelated [`Expr::Subquery`] of an expression with the
/// literal value it produced. A correlated one is left in place, because its
/// value depends on the row being projected.
fn resolve_scalar_subqueries(
    expr: &Expr,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
) -> Result<Expr> {
    let resolve = |inner: &Expr| resolve_scalar_subqueries(inner, contract, records, bound_values);
    Ok(match expr {
        Expr::Subquery { ops, correlated } => {
            if *correlated {
                expr.clone()
            }
            else {
                let value = scalar_subquery_value(ops, contract, records, bound_values, false, None)?;
                Expr::Bound(BoundValue::Literal(value))
            }
        }
        Expr::Case { branches, else_branch } => {
            let mut resolved = Vec::with_capacity(branches.len());
            for (when, then) in branches {
                resolved.push((Box::new(resolve(when)?), Box::new(resolve(then)?)));
            }
            let else_branch = else_branch.as_ref().map(|els| resolve(els)).transpose()?.map(Box::new);
            Expr::Case { branches: resolved, else_branch }
        }
        Expr::Cast { expr: inner, target } => Expr::Cast { expr: Box::new(resolve(inner)?), target: target.clone() },
        Expr::Binary { op, left, right } => {
            Expr::Binary { op: *op, left: Box::new(resolve(left)?), right: Box::new(resolve(right)?) }
        }
        Expr::Not(inner) => Expr::Not(Box::new(resolve(inner)?)),
        Expr::Function { function, args } => {
            let mut resolved = Vec::with_capacity(args.len());
            for arg in args {
                resolved.push(resolve(arg)?);
            }
            Expr::Function { function: *function, args: resolved }
        }
        other => other.clone(),
    })
}

/// `CASE WHEN w1 THEN t1 WHEN w2 THEN t2 ... [ELSE e] END`.
///
/// SQL semantics: evaluate each WHEN left-to-right; the first one that is
/// truthy (non-NULL, non-false boolean) yields the corresponding THEN. If no
/// WHEN matches, the ELSE branch is taken; absent ELSE yields NULL.
fn eval_case(
    branches: &[(Box<Expr>, Box<Expr>)],
    else_branch: &Option<Box<Expr>>,
    row: &QueryRow,
    table: &str,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    outer: Option<&OuterRow<'_>>,
) -> Result<Value> {
    for (when, then) in branches {
        let when_val = eval_expr(when, row, table, contract, records, bound_values, outer)?;
        if is_truthy(&when_val) {
            return eval_expr(then, row, table, contract, records, bound_values, outer);
        }
    }
    match else_branch {
        Some(els) => eval_expr(els, row, table, contract, records, bound_values, outer),
        None => Ok(Value::Null),
    }
}

/// SQL truthiness used for CASE WHEN predicates.
///
/// - `TRUE` / non-zero numeric → truthy
/// - `FALSE` / zero numeric / empty text / NULL → falsy
fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Null => false,
        Value::I64(n) => *n != 0,
        Value::U64(n) => *n != 0,
        Value::F64(n) => *n != 0.0,
        Value::Text(s) => !s.is_empty(),
        Value::Bytes(b) => !b.is_empty(),
        _ => true,
    }
}

/// Evaluate a binary expression (arithmetic, comparison or logical) for CASE
/// WHEN predicates, arithmetic projections and nested scalar comparisons.
///
/// NULL propagation follows SQL 3VL semantics: arithmetic and comparisons yield
/// `NULL` when either operand is `NULL`, and AND/OR treat `NULL` as unknown.
fn eval_binary(op: BinaryOp, left: &Value, right: &Value) -> Result<Value> {
    use BinaryOp::*;
    match op {
        // AND / OR use SQL truth tables with NULL propagation.
        And => Ok(match (left, right) {
            (Value::Bool(false), _) | (_, Value::Bool(false)) => Value::Bool(false),
            (Value::Null, _) | (_, Value::Null) => Value::Null,
            _ => Value::Bool(is_truthy(left) && is_truthy(right)),
        }),
        Or => Ok(match (left, right) {
            (Value::Bool(true), _) | (_, Value::Bool(true)) => Value::Bool(true),
            (Value::Null, _) | (_, Value::Null) => Value::Null,
            _ => Value::Bool(is_truthy(left) || is_truthy(right)),
        }),
        Add | Sub | Mul | Div | Rem => eval_arithmetic(op, left, right),
        // Comparisons return NULL when either operand is NULL.
        _ => {
            if matches!(left, Value::Null) || matches!(right, Value::Null) {
                return Ok(Value::Null);
            }
            let cmp = compare_values(left, right);
            let result = match op {
                Eq => cmp == std::cmp::Ordering::Equal,
                Ne => cmp != std::cmp::Ordering::Equal,
                Lt => cmp == std::cmp::Ordering::Less,
                Le => cmp != std::cmp::Ordering::Greater,
                Gt => cmp == std::cmp::Ordering::Greater,
                Ge => cmp != std::cmp::Ordering::Less,
                And | Or | Add | Sub | Mul | Div | Rem => unreachable!(),
            };
            Ok(Value::Bool(result))
        }
    }
}

/// Arithmetic type rule for projections and aggregate arguments.
///
/// * A `NULL` operand yields `NULL`, so the result never invents a number.
/// * Two integer operands (`i64`/`u64`) stay integers: the result is
///   [`Value::I64`] and an overflow is an error instead of a silent wrap. A
///   `u64` operand outside the signed range is rejected for the same reason.
/// * Any REAL operand (`f64`) promotes both sides to `f64` and the result is
///   [`Value::F64`]. A non-finite result is an error, matching the literal rule.
/// * Division or remainder by zero yields `NULL` for both numeric families.
/// * Any other operand kind is an error: YYDB never coerces text or bytes into
///   a number here.
fn eval_arithmetic(op: BinaryOp, left: &Value, right: &Value) -> Result<Value> {
    if matches!(left, Value::Null) || matches!(right, Value::Null) {
        return Ok(Value::Null);
    }
    match (left, right) {
        (Value::I64(a), Value::I64(b)) => integer_arithmetic(op, *a, *b),
        (Value::I64(a), Value::U64(b)) => integer_arithmetic(op, *a, signed_u64(*b)?),
        (Value::U64(a), Value::I64(b)) => integer_arithmetic(op, signed_u64(*a)?, *b),
        (Value::U64(a), Value::U64(b)) => integer_arithmetic(op, signed_u64(*a)?, signed_u64(*b)?),
        (Value::I64(a), Value::F64(b)) => real_arithmetic(op, *a as f64, *b),
        (Value::U64(a), Value::F64(b)) => real_arithmetic(op, *a as f64, *b),
        (Value::F64(a), Value::I64(b)) => real_arithmetic(op, *a, *b as f64),
        (Value::F64(a), Value::U64(b)) => real_arithmetic(op, *a, *b as f64),
        (Value::F64(a), Value::F64(b)) => real_arithmetic(op, *a, *b),
        _ => Err(query_runtime_error(format!(
            "arithmetic operator `{}` requires numeric operands, got `{}` and `{}`",
            arithmetic_symbol(op),
            left.kind(),
            right.kind()
        ))),
    }
}

/// Narrow a `u64` operand into the signed range the result type uses.
fn signed_u64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| {
        query_runtime_error(format!("integer operand `{value}` is outside the signed 64-bit range"))
    })
}

fn integer_arithmetic(op: BinaryOp, left: i64, right: i64) -> Result<Value> {
    let checked = match op {
        BinaryOp::Add => left.checked_add(right),
        BinaryOp::Sub => left.checked_sub(right),
        BinaryOp::Mul => left.checked_mul(right),
        BinaryOp::Div => {
            if right == 0 {
                return Ok(Value::Null);
            }
            left.checked_div(right)
        }
        BinaryOp::Rem => {
            if right == 0 {
                return Ok(Value::Null);
            }
            left.checked_rem(right)
        }
        _ => unreachable!("integer_arithmetic only receives arithmetic operators"),
    };
    checked.map(Value::I64).ok_or_else(|| {
        query_runtime_error(format!(
            "integer arithmetic `{}` overflowed the signed 64-bit range",
            arithmetic_symbol(op)
        ))
    })
}

fn real_arithmetic(op: BinaryOp, left: f64, right: f64) -> Result<Value> {
    let result = match op {
        BinaryOp::Add => left + right,
        BinaryOp::Sub => left - right,
        BinaryOp::Mul => left * right,
        BinaryOp::Div => {
            if right == 0.0 {
                return Ok(Value::Null);
            }
            left / right
        }
        BinaryOp::Rem => {
            if right == 0.0 {
                return Ok(Value::Null);
            }
            left % right
        }
        _ => unreachable!("real_arithmetic only receives arithmetic operators"),
    };
    if !result.is_finite() {
        return Err(query_runtime_error(format!(
            "REAL arithmetic `{}` produced a non-finite result",
            arithmetic_symbol(op)
        )));
    }
    Ok(Value::F64(result))
}

fn arithmetic_symbol(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Rem => "%",
        _ => unreachable!("arithmetic_symbol only receives arithmetic operators"),
    }
}

/// Evaluate one scalar function call.
///
/// Every function propagates `NULL`: a `NULL` argument, including a `NULL`
/// inside `CONCAT`, makes the whole result `NULL`, except `COALESCE`, which
/// returns its first non-`NULL` argument. String functions require text
/// arguments (or integer positions for `SUBSTR`) and the date part extractors
/// require a `date` or `datetime` — any other kind is an error rather than an
/// implicit conversion.
fn eval_function(
    function: ScalarFunction,
    args: &[Expr],
    row: &QueryRow,
    table: &str,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    outer: Option<&OuterRow<'_>>,
) -> Result<Value> {
    let mut values = Vec::with_capacity(args.len());
    for arg in args {
        values.push(eval_expr(arg, row, table, contract, records, bound_values, outer)?);
    }
    // `COALESCE` is the one function that a `NULL` argument does not decide, so
    // it skips the blanket propagation and picks the first non-`NULL` value.
    if !matches!(function, ScalarFunction::Coalesce)
        && values.iter().any(|value| matches!(value, Value::Null))
    {
        return Ok(Value::Null);
    }
    Ok(match function {
        ScalarFunction::Lower => Value::Text(text_argument(function, &values[0])?.to_lowercase()),
        ScalarFunction::Upper => Value::Text(text_argument(function, &values[0])?.to_uppercase()),
        ScalarFunction::Trim => Value::Text(text_argument(function, &values[0])?.trim().to_string()),
        ScalarFunction::Length | ScalarFunction::CharLength => {
            Value::I64(text_argument(function, &values[0])?.chars().count() as i64)
        }
        ScalarFunction::Substr => substr(&values)?,
        ScalarFunction::Replace => {
            let text = text_argument(function, &values[0])?;
            let from = text_argument(function, &values[1])?;
            let to = text_argument(function, &values[2])?;
            // `str::replace` with an empty needle inserts `to` between every
            // character, so an empty `from` must return the input unchanged.
            Value::Text(if from.is_empty() { text.to_string() } else { text.replace(from, to) })
        }
        ScalarFunction::Concat => {
            let mut joined = String::new();
            for value in &values {
                joined.push_str(text_argument(function, value)?);
            }
            Value::Text(joined)
        }
        ScalarFunction::Year | ScalarFunction::Month | ScalarFunction::Day => {
            Value::I64(date_part(function, &values[0])? as i64)
        }
        ScalarFunction::Coalesce => coalesce(&values)?,
    })
}

/// `COALESCE(a, b, …)`: the first non-`NULL` argument, or `NULL` when every
/// argument is `NULL`.
///
/// The result kind is decided at execution time, and every non-`NULL` argument
/// must share one value kind: `COALESCE(text, i64)` is an error rather than an
/// implicit conversion, because the returned value is passed through unchanged.
fn coalesce(args: &[Value]) -> Result<Value> {
    let mut selected: Option<&Value> = None;
    for value in args {
        if matches!(value, Value::Null) {
            continue;
        }
        match selected {
            Some(first) if first.kind() != value.kind() => {
                return Err(query_runtime_error(format!(
                    "`COALESCE()` arguments must share one value kind, got `{}` and `{}`",
                    first.kind(),
                    value.kind()
                )));
            }
            Some(_) => {}
            None => selected = Some(value),
        }
    }
    Ok(selected.cloned().unwrap_or(Value::Null))
}

/// Extract one calendar part from a `date` or `datetime` value.
///
/// A `datetime` contributes its UTC calendar date exactly as stored: no timezone
/// is applied, because the value carries none. Anything else — text, `time`,
/// numbers — is rejected rather than parsed with an assumed format.
fn date_part(function: ScalarFunction, value: &Value) -> Result<i32> {
    let (year, month, day) = match value {
        Value::Date(date) => date.to_ymd(),
        Value::Timestamp(timestamp) => {
            let (date, _) = timestamp.to_parts();
            date.to_ymd()
        }
        other => {
            return Err(query_runtime_error(format!(
                "`{}()` requires a `date` or `datetime` argument, got `{}`",
                function_name(function),
                other.kind()
            )));
        }
    };
    Ok(match function {
        ScalarFunction::Year => year,
        ScalarFunction::Month => month as i32,
        _ => day as i32,
    })
}

fn text_argument(function: ScalarFunction, value: &Value) -> Result<&str> {
    match value {
        Value::Text(text) => Ok(text.as_str()),
        other => Err(query_runtime_error(format!(
            "`{}()` requires text arguments, got `{}`",
            function_name(function),
            other.kind()
        ))),
    }
}

fn integer_argument(function: ScalarFunction, value: &Value) -> Result<i64> {
    match value {
        Value::I64(number) => Ok(*number),
        Value::U64(number) => signed_u64(*number),
        other => Err(query_runtime_error(format!(
            "`{}()` requires integer position arguments, got `{}`",
            function_name(function),
            other.kind()
        ))),
    }
}

/// `SUBSTR(text, start[, length])` over Unicode characters.
///
/// `start` is 1-based, and a negative `start` counts from the end (`-1` is the
/// last character). Zero has no unambiguous meaning and is rejected instead of
/// guessing, as is a negative `length`. A slice that falls outside the string
/// yields an empty string.
fn substr(args: &[Value]) -> Result<Value> {
    let text = text_argument(ScalarFunction::Substr, &args[0])?;
    let start = integer_argument(ScalarFunction::Substr, &args[1])?;
    if start == 0 {
        return Err(query_runtime_error(
            "`SUBSTR` start position must not be 0: positions are 1-based and negative positions count from the end",
        ));
    }
    let length = args.get(2).map(|value| integer_argument(ScalarFunction::Substr, value)).transpose()?;
    if let Some(length) = length
        && length < 0
    {
        return Err(query_runtime_error("`SUBSTR` length must not be negative"));
    }
    let count = text.chars().count() as i64;
    let begin = if start > 0 { start - 1 } else { count + start }.clamp(0, count);
    let end = match length {
        Some(length) => (begin + length).clamp(begin, count),
        None => count,
    };
    Ok(Value::Text(text.chars().skip(begin as usize).take((end - begin) as usize).collect()))
}

fn function_name(function: ScalarFunction) -> &'static str {
    match function {
        ScalarFunction::Lower => "LOWER",
        ScalarFunction::Upper => "UPPER",
        ScalarFunction::Length => "LENGTH",
        ScalarFunction::CharLength => "CHAR_LENGTH",
        ScalarFunction::Substr => "SUBSTR",
        ScalarFunction::Trim => "TRIM",
        ScalarFunction::Replace => "REPLACE",
        ScalarFunction::Coalesce => "COALESCE",
        ScalarFunction::Concat => "CONCAT",
        ScalarFunction::Year => "YEAR",
        ScalarFunction::Month => "MONTH",
        ScalarFunction::Day => "DAY",
    }
}

/// `CAST(expr AS <target>)` supporting the four standard SQL target types.
///
/// Returns an `Err` when the conversion is not representable (e.g. text that
/// does not parse as integer) so the caller can surface a diagnostic rather
/// than silently producing an incorrect value. Casting `NULL` preserves NULL.
fn eval_cast(value: &Value, target: &str) -> Result<Value> {
    // NULL stays NULL regardless of target type.
    if matches!(value, Value::Null) {
        return Ok(Value::Null);
    }
    let target = target.trim().to_ascii_uppercase();
    match target.as_str() {
        "INTEGER" | "INT" | "BIGINT" => cast_to_integer(value),
        "REAL" | "FLOAT" | "DOUBLE" => cast_to_real(value),
        "TEXT" | "VARCHAR" | "CHAR" | "STRING" => cast_to_text(value),
        "BLOB" | "BYTEA" | "BYTES" => cast_to_blob(value),
        other => Err(query_runtime_error(format!("unsupported CAST target type `{other}`"))),
    }
}

fn cast_to_integer(value: &Value) -> Result<Value> {
    let n = match value {
        Value::Bool(b) => if *b { 1 } else { 0 },
        Value::I64(n) => return Ok(Value::I64(*n)),
        Value::U64(n) => {
            let clamped = (*n).min(i64::MAX as u64) as i64;
            return Ok(Value::I64(clamped));
        }
        Value::F64(n) => {
            if n.is_nan() {
                return Err(query_runtime_error("CAST NaN to INTEGER is not representable"));
            }
            *n as i64
        }
        Value::Text(s) => s
            .trim()
            .parse::<i64>()
            .map_err(|_| query_runtime_error(format!("CAST text `{s}` to INTEGER failed")))?,
        _ => return Err(query_runtime_error(format!("CAST from `{:?}` to INTEGER is not supported", value))),
    };
    Ok(Value::I64(n))
}

fn cast_to_real(value: &Value) -> Result<Value> {
    let n = match value {
        Value::Bool(b) => if *b { 1.0 } else { 0.0 },
        Value::I64(n) => *n as f64,
        Value::U64(n) => *n as f64,
        Value::F64(n) => return Ok(Value::F64(*n)),
        Value::Text(s) => s
            .trim()
            .parse::<f64>()
            .map_err(|_| query_runtime_error(format!("CAST text `{s}` to REAL failed")))?,
        _ => return Err(query_runtime_error(format!("CAST from `{:?}` to REAL is not supported", value))),
    };
    Ok(Value::F64(n))
}

fn cast_to_text(value: &Value) -> Result<Value> {
    let text = match value {
        Value::Null => return Ok(Value::Null),
        Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Value::I64(n) => n.to_string(),
        Value::U64(n) => n.to_string(),
        Value::F64(n) => {
            // Use the canonical float representation, trimmed of trailing zeros.
            // Format specifier `.15` matches SQLite-like display precision.
            let s = format!("{n:.15}");
            let trimmed = s.trim_end_matches('0').trim_end_matches('.').to_string();
            if trimmed.contains('.') {
                trimmed
            }
            else {
                format!("{trimmed}.0")
            }
        }
        Value::Text(s) => return Ok(Value::Text(s.clone())),
        Value::Uuid(u) => u.to_string(),
        _ => return Err(query_runtime_error(format!("CAST from `{:?}` to TEXT is not supported", value))),
    };
    Ok(Value::Text(text))
}

fn cast_to_blob(value: &Value) -> Result<Value> {
    let bytes = match value {
        Value::Null => return Ok(Value::Null),
        Value::Bytes(b) => return Ok(Value::Bytes(b.clone())),
        Value::Text(s) => s.as_bytes().to_vec(),
        _ => return Err(query_runtime_error(format!("CAST from `{:?}` to BLOB is not supported", value))),
    };
    Ok(Value::Bytes(bytes))
}

// ─── join ────────────────────────────────────────────────────────────────

/// Merge the left row stream with the rows the right source already produced.
///
/// Both sides arrive as plain rows, so a stored table and a derived table share
/// one join implementation and one prefix rule.
fn execute_join(
    kind: JoinKind,
    left_rows: &[QueryRow],
    right_rows: &[QueryRow],
    prefix: &str,
    on: &Pred,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    outer: Option<&OuterRow<'_>>,
) -> Result<Vec<QueryRow>> {
    // The `ON` condition is evaluated per candidate pair. A subquery inside it
    // that reads the pair is therefore run per pair, while one that reads the
    // enclosing statement's row still resolves against `outer`.
    match kind {
        JoinKind::Inner => execute_inner_join(left_rows, right_rows, prefix, on, contract, records, bound_values, outer),
        JoinKind::Left => execute_left_join(left_rows, right_rows, prefix, on, contract, records, bound_values, outer),
        JoinKind::Right => execute_right_join(left_rows, right_rows, prefix, on, contract, records, bound_values, outer),
        JoinKind::Full => execute_full_join(left_rows, right_rows, prefix, on, contract, records, bound_values, outer),
    }
}

fn execute_inner_join(
    left_rows: &[QueryRow],
    right_rows: &[QueryRow],
    prefix: &str,
    on: &Pred,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    outer: Option<&OuterRow<'_>>,
) -> Result<Vec<QueryRow>> {
    // Nested loop: for each left row, for each right row, check ON condition.
    // The ON predicate operates on a merged row where both sides are
    // addressable via their table-qualified column names (prefix.column).
    let mut out: Vec<QueryRow> = Vec::with_capacity(left_rows.len().min(right_rows.len()));
    for left_row in left_rows {
        for right_row in right_rows {
            let merged = merge_rows(left_row, right_row, prefix);
            // JOIN ON condition — eval_pred reads directly from the merged
            // row by table-qualified column name, so `table` is irrelevant
            // for single-element paths.
            if eval_pred_for_row(on, &merged, "", contract, records, bound_values, outer)? {
                out.push(merged);
            }
        }
    }
    Ok(out)
}

fn execute_left_join(
    left_rows: &[QueryRow],
    right_rows: &[QueryRow],
    prefix: &str,
    on: &Pred,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    outer: Option<&OuterRow<'_>>,
) -> Result<Vec<QueryRow>> {
    // Qualified column names that belong to the right side so we can pad
    // unmatched left rows with NULL for each right column.
    let right_col_names: Vec<String> =
        right_rows.first().map(|r| r.keys().map(|k| format!("{prefix}.{k}")).collect()).unwrap_or_default();

    // Track which left rows had at least one right-side match.
    let mut matched = vec![false; left_rows.len()];
    let mut out: Vec<QueryRow> = Vec::with_capacity(left_rows.len());

    // First pass: emit merged rows that satisfy the ON condition and mark
    // the contributing left row as matched.
    for (li, left_row) in left_rows.iter().enumerate() {
        for right_row in right_rows {
            let merged = merge_rows(left_row, right_row, prefix);
            if eval_pred_for_row(on, &merged, "", contract, records, bound_values, outer)? {
                matched[li] = true;
                out.push(merged);
            }
        }
    }

    // Second pass: for every left row that never matched, emit a padded row
    // carrying the left columns plus NULL for each right column.
    for (li, left_row) in left_rows.iter().enumerate() {
        if !matched[li] {
            let mut padded = left_row.clone();
            for col in &right_col_names {
                padded.insert(col.clone(), Value::Null);
            }
            out.push(padded);
        }
    }

    Ok(out)
}

fn execute_right_join(
    left_rows: &[QueryRow],
    right_rows: &[QueryRow],
    prefix: &str,
    on: &Pred,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    outer: Option<&OuterRow<'_>>,
) -> Result<Vec<QueryRow>> {
    // Bare column names of the base (left) table, used to pad right rows that
    // never matched. They are never prefixed, so the padded row has the same
    // shape as a matched row.
    let left_col_names: Vec<String> = left_rows.first().map(|r| r.keys().cloned().collect()).unwrap_or_default();

    // Track which right rows had at least one left-side match.
    let mut matched = vec![false; right_rows.len()];
    let mut out: Vec<QueryRow> = Vec::with_capacity(right_rows.len());

    // First pass: emit merged rows that satisfy the ON condition and mark
    // the contributing right row as matched.
    for left_row in left_rows {
        for (ri, right_row) in right_rows.iter().enumerate() {
            let merged = merge_rows(left_row, right_row, prefix);
            if eval_pred_for_row(on, &merged, "", contract, records, bound_values, outer)? {
                matched[ri] = true;
                out.push(merged);
            }
        }
    }

    // Second pass: for every right row that never matched, emit a padded row
    // carrying the prefixed right columns plus NULL for each left column.
    for (ri, right_row) in right_rows.iter().enumerate() {
        if !matched[ri] {
            let mut padded = QueryRow::new();
            for col in &left_col_names {
                padded.insert(col.clone(), Value::Null);
            }
            for (key, value) in right_row {
                padded.insert(format!("{prefix}.{key}"), value.clone());
            }
            out.push(padded);
        }
    }

    Ok(out)
}

fn execute_full_join(
    left_rows: &[QueryRow],
    right_rows: &[QueryRow],
    prefix: &str,
    on: &Pred,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    outer: Option<&OuterRow<'_>>,
) -> Result<Vec<QueryRow>> {
    // Column names of each side, used to pad rows that never matched.
    let left_col_names: Vec<String> = left_rows.first().map(|r| r.keys().cloned().collect()).unwrap_or_default();
    let right_col_names: Vec<String> =
        right_rows.first().map(|r| r.keys().map(|k| format!("{prefix}.{k}")).collect()).unwrap_or_default();

    // Track which rows on either side had at least one match.
    let mut left_matched = vec![false; left_rows.len()];
    let mut right_matched = vec![false; right_rows.len()];
    let mut out: Vec<QueryRow> = Vec::with_capacity(left_rows.len() + right_rows.len());

    // First pass: emit every merged row that satisfies the ON condition.
    for (li, left_row) in left_rows.iter().enumerate() {
        for (ri, right_row) in right_rows.iter().enumerate() {
            let merged = merge_rows(left_row, right_row, prefix);
            if eval_pred_for_row(on, &merged, "", contract, records, bound_values, outer)? {
                left_matched[li] = true;
                right_matched[ri] = true;
                out.push(merged);
            }
        }
    }

    // Second pass: unmatched left rows get NULL right columns.
    for (li, left_row) in left_rows.iter().enumerate() {
        if !left_matched[li] {
            let mut padded = left_row.clone();
            for col in &right_col_names {
                padded.insert(col.clone(), Value::Null);
            }
            out.push(padded);
        }
    }

    // Third pass: unmatched right rows get NULL left columns.
    for (ri, right_row) in right_rows.iter().enumerate() {
        if !right_matched[ri] {
            let mut padded = QueryRow::new();
            for col in &left_col_names {
                padded.insert(col.clone(), Value::Null);
            }
            for (key, value) in right_row {
                padded.insert(format!("{prefix}.{key}"), value.clone());
            }
            out.push(padded);
        }
    }

    Ok(out)
}

/// Merge a left and right row. Left columns are kept as-is; right columns
/// are prefixed with `prefix.column_name`.
fn merge_rows(left: &QueryRow, right: &QueryRow, prefix: &str) -> QueryRow {
    let mut merged = QueryRow::new();
    for (key, value) in left {
        merged.insert(key.clone(), value.clone());
    }
    for (key, value) in right {
        let qualified = format!("{prefix}.{key}");
        merged.insert(qualified, value.clone());
    }
    merged
}

// ─── set operations ──────────────────────────────────────────────────────

/// Combine both operands of a set operation.
///
/// Both pipelines are materialized first, so neither operand can observe the
/// other's rows. The result keeps the left operand's column names: the right
/// rows are aligned positionally onto them, which is why both operands must
/// produce the same number of columns and compatible column types.
///
/// `UNION`, `INTERSECT` and `EXCEPT` treat two `NULL`s as duplicates of each
/// other (SQL set semantics), so de-duplication compares the aligned values of
/// each row rather than using `=` on individual columns.
fn execute_set_operation(
    kind: SetOpKind,
    left_ops: &[QueryOp],
    right_ops: &[QueryOp],
    columns: Option<&[String]>,
    right_columns: Option<&[String]>,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    outer: Option<&OuterRow<'_>>,
) -> Result<Vec<QueryRow>> {
    let left_rows = execute_ops_in(left_ops, contract, records, bound_values, outer)?;
    let right_rows = execute_ops_in(right_ops, contract, records, bound_values, outer)?;

    let mut left_columns = match columns {
        Some(columns) => columns.to_vec(),
        None => first_row_columns(&left_rows),
    };
    let right_columns = match right_columns {
        Some(columns) => columns.to_vec(),
        None => first_row_columns(&right_rows),
    };
    // A `*` operand with no rows carries no column names of its own. Nothing is
    // aligned in that case — the other operand's names are the only ones left.
    if left_columns.is_empty() {
        left_columns = right_columns.clone();
    }
    if left_columns.len() != right_columns.len() {
        return Err(query_runtime_error(format!(
            "{} operands must produce the same number of columns: {} on the left, {} on the right",
            kind.as_str(),
            left_columns.len(),
            right_columns.len()
        )));
    }
    check_set_operation_types(kind, &left_rows, &right_rows, &left_columns, &right_columns)?;

    let aligned_right: Vec<QueryRow> =
        right_rows.iter().map(|row| align_row(row, &right_columns, &left_columns)).collect();
    Ok(match kind {
        SetOpKind::UnionAll => left_rows.into_iter().chain(aligned_right).collect(),
        SetOpKind::Union => {
            let mut out: Vec<QueryRow> = Vec::with_capacity(left_rows.len() + aligned_right.len());
            let mut seen: BTreeSet<String> = BTreeSet::new();
            for row in left_rows.into_iter().chain(aligned_right) {
                if seen.insert(set_row_key(&row, &left_columns)) {
                    out.push(row);
                }
            }
            out
        }
        SetOpKind::Intersect | SetOpKind::Except => {
            let right_keys: BTreeSet<String> =
                aligned_right.iter().map(|row| set_row_key(row, &left_columns)).collect();
            let mut out: Vec<QueryRow> = Vec::new();
            let mut seen: BTreeSet<String> = BTreeSet::new();
            for row in left_rows {
                let key = set_row_key(&row, &left_columns);
                let keep = if matches!(kind, SetOpKind::Intersect) {
                    right_keys.contains(&key)
                }
                else {
                    !right_keys.contains(&key)
                };
                if keep && seen.insert(key) {
                    out.push(row);
                }
            }
            out
        }
    })
}

/// Column names of the first row, i.e. the `*` shape of an executed operand.
fn first_row_columns(rows: &[QueryRow]) -> Vec<String> {
    rows.first().map(|row| row.keys().cloned().collect()).unwrap_or_default()
}

/// Map one row of the right operand onto the left operand's column names by
/// position, filling a missing value with `NULL`.
fn align_row(row: &QueryRow, from: &[String], to: &[String]) -> QueryRow {
    let mut out = QueryRow::new();
    for (index, column) in to.iter().enumerate() {
        let value = from.get(index).and_then(|source| row.get(source)).cloned().unwrap_or(Value::Null);
        out.insert(column.clone(), value);
    }
    out
}

/// Canonical text key of a row's values in the given column order. `NULL`
/// compares equal to `NULL` here because a set operation removes duplicate rows
/// rather than evaluating `=` per column.
fn set_row_key(row: &QueryRow, columns: &[String]) -> String {
    let values: Vec<Value> = columns.iter().map(|column| row.get(column).cloned().unwrap_or(Value::Null)).collect();
    group_key_to_string(&values)
}

/// Reject a set operation whose operands disagree on one column's type.
///
/// Lowering has no catalog, so the types are read from the executed values.
/// `NULL` is compatible with every type; `I64` and `U64` are one integer type.
fn check_set_operation_types(
    kind: SetOpKind,
    left_rows: &[QueryRow],
    right_rows: &[QueryRow],
    left_columns: &[String],
    right_columns: &[String],
) -> Result<()> {
    let mut kinds: Vec<Option<&'static str>> = vec![None; left_columns.len()];
    for (rows, columns) in [(left_rows, left_columns), (right_rows, right_columns)] {
        for row in rows {
            for (index, column) in columns.iter().enumerate() {
                let Some(label) = row.get(column).and_then(value_type_label)
                else {
                    continue;
                };
                match kinds[index] {
                    None => kinds[index] = Some(label),
                    Some(existing) if existing == label => {}
                    Some(existing) => {
                        return Err(query_runtime_error(format!(
                            "{} column {} (`{}`) mixes `{existing}` and `{label}` values: both operands must produce the same type",
                            kind.as_str(),
                            index + 1,
                            left_columns[index]
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Scalar type label of one value, or `None` for `NULL` (compatible with any).
fn value_type_label(value: &Value) -> Option<&'static str> {
    match value {
        Value::Null => None,
        Value::I64(_) | Value::U64(_) => Some("integer"),
        Value::F64(_) => Some("real"),
        other => Some(other.kind()),
    }
}

// ─── aggregate ───────────────────────────────────────────────────────────

fn execute_aggregate(
    rows: &[QueryRow],
    group_keys: &[ProjectField],
    aggregates: &[AggregateField],
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    outer: Option<&OuterRow<'_>>,
) -> Result<Vec<QueryRow>> {
    if group_keys.is_empty() {
        // Global aggregate: all rows form one group.
        let mut out = QueryRow::new();
        for agg in aggregates {
            let (name, value) = eval_aggregate(agg, rows, contract, records, bound_values, outer)?;
            out.insert(name, value);
        }
        return Ok(vec![out]);
    }

    // Group rows by the group key values using a string key (Value is not
    // Hash/Eq). We generate a canonical key from each row's GROUP BY values.
    let mut groups: HashMap<String, Vec<&QueryRow>> = HashMap::new();
    for row in rows {
        let mut key_values: Vec<Value> = Vec::with_capacity(group_keys.len());
        for gk in group_keys {
            let value = match &gk.expr {
                ProjectExpr::Scalar { path } => read_path(path, row, "", contract, records, outer),
                ProjectExpr::Nested { .. } => Value::Null, // nested in GROUP BY → unsupported
                ProjectExpr::Computed(_) => Value::Null, // computed in GROUP BY → unsupported (lowering rejects)
            };
            key_values.push(value);
        }
        let group_key = group_key_to_string(&key_values);
        groups.entry(group_key).or_default().push(row);
    }

    let mut out_rows: Vec<QueryRow> = Vec::with_capacity(groups.len());
    for (_, group_rows) in &groups {
        let mut out = QueryRow::new();
        // Group key columns — read from the first row of the group.
        if let Some(first_row) = group_rows.first() {
            for gk in group_keys {
                let value = match &gk.expr {
                    ProjectExpr::Scalar { path } => read_path(path, first_row, "", contract, records, outer),
                    ProjectExpr::Nested { .. } => Value::Null,
                    ProjectExpr::Computed(_) => Value::Null,
                };
                out.insert(gk.name.clone(), value);
            }
        }
        // Aggregate values.
        let group_row_vec: Vec<QueryRow> = group_rows.iter().cloned().cloned().collect();
        for agg in aggregates {
            let (name, value) = eval_aggregate(agg, &group_row_vec, contract, records, bound_values, outer)?;
            out.insert(name, value);
        }
        out_rows.push(out);
    }
    Ok(out_rows)
}

fn eval_aggregate(
    agg: &AggregateField,
    group_rows: &[QueryRow],
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    outer: Option<&OuterRow<'_>>,
) -> Result<(String, Value)> {
    match agg {
        AggregateField::Count { name, expr: None } => {
            // COUNT(*) — number of rows in the group.
            Ok((name.clone(), Value::I64(group_rows.len() as i64)))
        }
        AggregateField::Count { name, expr: Some(inner) } => {
            // COUNT(expr) — non-null values of the expression.
            let mut count = 0_i64;
            for row in group_rows {
                if !matches!(eval_project_expr(inner, row, contract, records, bound_values, outer)?, Value::Null) {
                    count += 1;
                }
            }
            Ok((name.clone(), Value::I64(count)))
        }
        AggregateField::CountDistinct { name, expr } => {
            // COUNT(DISTINCT expr) — distinct non-null values of the expression.
            // Value implements neither Hash nor Ord so we serialise each
            // non-null value to its Debug form for set membership. MVP scalar
            // values (Null, Bool, I64, Text, Uuid …) are all uniquely
            // represented by their Debug output.
            let mut seen: BTreeSet<String> = BTreeSet::new();
            for row in group_rows {
                let value = eval_project_expr(expr, row, contract, records, bound_values, outer)?;
                if !matches!(value, Value::Null) {
                    seen.insert(format!("{value:?}"));
                }
            }
            Ok((name.clone(), Value::I64(seen.len() as i64)))
        }
        AggregateField::Sum { name, expr } => {
            // SUM keeps the numeric family it was given: integer values
            // accumulate exactly and yield `I64`, and the first REAL value
            // promotes the whole sum to `F64`. A group with no numeric value is
            // `NULL`, and a non-numeric value is an error rather than being
            // skipped.
            let mut integer_sum: i128 = 0;
            let mut real_sum: f64 = 0.0;
            let mut is_real = false;
            let mut saw_any = false;
            for row in group_rows {
                match eval_project_expr(expr, row, contract, records, bound_values, outer)? {
                    Value::Null => {}
                    Value::I64(n) => {
                        integer_sum += i128::from(n);
                        real_sum += n as f64;
                        saw_any = true;
                    }
                    Value::U64(n) => {
                        integer_sum += i128::from(n);
                        real_sum += n as f64;
                        saw_any = true;
                    }
                    Value::F64(n) => {
                        is_real = true;
                        real_sum += n;
                        saw_any = true;
                    }
                    other => {
                        return Err(query_runtime_error(format!(
                            "SUM() requires numeric values, got `{}`",
                            other.kind()
                        )));
                    }
                }
            }
            let result = if !saw_any {
                Value::Null
            }
            else if is_real {
                finite_real(real_sum, "SUM()")?
            }
            else {
                let sum = i64::try_from(integer_sum).map_err(|_| {
                    query_runtime_error("SUM() overflowed the signed 64-bit range")
                })?;
                Value::I64(sum)
            };
            Ok((name.clone(), result))
        }
        AggregateField::Avg { name, expr } => {
            // AVG always yields a finite `F64` (or `NULL` for an empty group)
            // because an average is not representable as an exact integer.
            let mut sum: f64 = 0.0;
            let mut count: u64 = 0;
            for row in group_rows {
                match eval_project_expr(expr, row, contract, records, bound_values, outer)? {
                    Value::Null => {}
                    Value::I64(n) => {
                        sum += n as f64;
                        count += 1;
                    }
                    Value::U64(n) => {
                        sum += n as f64;
                        count += 1;
                    }
                    Value::F64(n) => {
                        sum += n;
                        count += 1;
                    }
                    other => {
                        return Err(query_runtime_error(format!(
                            "AVG() requires numeric values, got `{}`",
                            other.kind()
                        )));
                    }
                }
            }
            let result = if count > 0 { finite_real(sum / count as f64, "AVG()")? } else { Value::Null };
            Ok((name.clone(), result))
        }
    }
}

/// Reject a non-finite REAL aggregate such as an overflowing `SUM`.
fn finite_real(value: f64, aggregate: &str) -> Result<Value> {
    if !value.is_finite() {
        return Err(query_runtime_error(format!("{aggregate} produced a non-finite REAL result")));
    }
    Ok(Value::F64(value))
}

/// Evaluate a `ProjectExpr` against a single row (used inside aggregate
/// evaluation where we have no table context).
///
/// Errors are propagated instead of falling back to `NULL`: an aggregate
/// argument that cannot be evaluated must fail the statement rather than
/// silently drop the row from the aggregate.
fn eval_project_expr(
    expr: &ProjectExpr,
    row: &QueryRow,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    outer: Option<&OuterRow<'_>>,
) -> Result<Value> {
    match expr {
        ProjectExpr::Scalar { path } => Ok(read_path(path, row, "", contract, records, outer)),
        ProjectExpr::Nested { path, .. } => Ok(read_path(path, row, "", contract, records, outer)),
        ProjectExpr::Computed(inner) => eval_expr(inner, row, "", contract, records, bound_values, outer),
    }
}

// ─── window ──────────────────────────────────────────────────────────────

/// Compute every window value and append it to the current rows.
///
/// The row order is preserved: a window only widens the rows, so a following
/// `Sort` still decides the result order. Each field is independent, which is
/// what lets one statement mix `ROW_NUMBER()` with a differently partitioned
/// `SUM`.
fn execute_window(
    rows: &[QueryRow],
    fields: &[WindowField],
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    outer: Option<&OuterRow<'_>>,
) -> Result<Vec<QueryRow>> {
    let mut rows: Vec<QueryRow> = rows.to_vec();
    for field in fields {
        // One value per input row, in the input order.
        let mut values: Vec<Value> = vec![Value::Null; rows.len()];
        for partition in partition_rows(&rows, field, contract, records, bound_values, outer)? {
            let values_of_partition =
                eval_window_partition(&rows, &partition, field, contract, records, bound_values, outer)?;
            for (row_index, value) in partition.into_iter().zip(values_of_partition) {
                values[row_index] = value;
            }
        }
        for (row, value) in rows.iter_mut().zip(values) {
            row.insert(field.name.clone(), value);
        }
    }
    Ok(rows)
}

/// Row indices of every partition, each in input order.
///
/// The partitions are keyed by the `PARTITION BY` values through the shared
/// canonical key form, so a partition is decided exactly like `GROUP BY` decides
/// a group.
fn partition_rows(
    rows: &[QueryRow],
    field: &WindowField,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    outer: Option<&OuterRow<'_>>,
) -> Result<Vec<Vec<usize>>> {
    if field.spec.partition_by.is_empty() {
        return Ok(vec![(0..rows.len()).collect()]);
    }
    let mut partitions: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, row) in rows.iter().enumerate() {
        let mut keys = Vec::with_capacity(field.spec.partition_by.len());
        for expr in &field.spec.partition_by {
            keys.push(eval_project_expr(expr, row, contract, records, bound_values, outer)?);
        }
        partitions.entry(group_key_to_string(&keys)).or_default().push(index);
    }
    Ok(partitions.into_values().collect())
}

/// Values one window field produces over one partition, aligned with the
/// partition's input row indices.
fn eval_window_partition(
    rows: &[QueryRow],
    partition: &[usize],
    field: &WindowField,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    outer: Option<&OuterRow<'_>>,
) -> Result<Vec<Value>> {
    // The window operates on the rows ordered by its own `ORDER BY`; the result
    // is written back to the input positions so the caller keeps its order.
    let mut keys: Vec<Vec<Value>> = Vec::with_capacity(partition.len());
    for &index in partition {
        keys.push(eval_window_order_keys(&rows[index], field, contract, records, bound_values, outer)?);
    }
    let mut order: Vec<usize> = (0..partition.len()).collect();
    if !field.spec.order_by.is_empty() {
        order.sort_by(|&left, &right| compare_window_keys(field, &keys[left], &keys[right]));
    }

    let mut values = vec![Value::Null; order.len()];
    if matches!(field.function, WindowFunction::RowNumber | WindowFunction::Rank | WindowFunction::DenseRank) {
        let mut dense_rank = 0_i64;
        let mut rank = 0_i64;
        for (position, &slot) in order.iter().enumerate() {
            // Peers are the adjacent rows whose `ORDER BY` values compare equal.
            let peer = position > 0 && compare_window_keys(field, &keys[order[position - 1]], &keys[slot]).is_eq();
            if !peer {
                dense_rank += 1;
                rank = position as i64 + 1;
            }
            values[slot] = Value::I64(match field.function {
                WindowFunction::RowNumber => position as i64 + 1,
                WindowFunction::Rank => rank,
                _ => dense_rank,
            });
        }
        return Ok(values);
    }

    for (position, &slot) in order.iter().enumerate() {
        let (start, end) = window_frame_range(field, &keys, &order, position);
        let frame_rows: Vec<QueryRow> = if start <= end {
            order[start..=end].iter().map(|&index| rows[partition[index]].clone()).collect()
        }
        else {
            Vec::new()
        };
        values[slot] = eval_window_aggregate(field, &frame_rows, contract, records, bound_values, outer)?;
    }
    Ok(values)
}

/// The `ORDER BY` values of one row, in key order.
fn eval_window_order_keys(
    row: &QueryRow,
    field: &WindowField,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    outer: Option<&OuterRow<'_>>,
) -> Result<Vec<Value>> {
    let mut keys = Vec::with_capacity(field.spec.order_by.len());
    for key in &field.spec.order_by {
        keys.push(eval_project_expr(&key.expr, row, contract, records, bound_values, outer)?);
    }
    Ok(keys)
}

/// Compare two `ORDER BY` value vectors with the shared value order.
///
/// The `NULL` placement is the one [`compare_values`] already defines and
/// [`QueryOp::Sort`] already uses, so a window and an `ORDER BY` never disagree
/// about where `NULL` rows belong.
fn compare_window_keys(field: &WindowField, left: &[Value], right: &[Value]) -> std::cmp::Ordering {
    for (key, (left_value, right_value)) in field.spec.order_by.iter().zip(left.iter().zip(right)) {
        let ord = compare_values(left_value, right_value);
        let ord = if key.ascending { ord } else { ord.reverse() };
        if ord != std::cmp::Ordering::Equal {
            return ord;
        }
    }
    std::cmp::Ordering::Equal
}

/// Inclusive frame bounds of `position` as indices into `order`.
///
/// Without an explicit frame the SQL default applies: the whole partition when
/// there is no `ORDER BY`, otherwise everything up to and including the current
/// row's last peer. An explicit `ROWS` frame counts rows from the current row
/// and is clamped to the partition, so an out-of-range bound never reads a row
/// outside it. A frame that starts after it ends is reported as `start > end`,
/// which the caller turns into the aggregate's empty-input result.
fn window_frame_range(field: &WindowField, keys: &[Vec<Value>], order: &[usize], position: usize) -> (usize, usize) {
    let len = order.len();
    let Some(frame) = &field.spec.frame else {
        if field.spec.order_by.is_empty() {
            return (0, len - 1);
        }
        let mut end = position;
        while end + 1 < len && compare_window_keys(field, &keys[order[position]], &keys[order[end + 1]]).is_eq() {
            end += 1;
        }
        return (0, end);
    };
    let start = frame_bound_index(&frame.start, position, len).max(0);
    let end = frame_bound_index(&frame.end, position, len).min(len as i64 - 1);
    if start > end {
        return (1, 0);
    }
    (start as usize, end as usize)
}

/// Index one frame bound denotes, before it is clamped to the partition.
fn frame_bound_index(bound: &WindowFrameBound, position: usize, len: usize) -> i64 {
    let position = position as i64;
    match bound {
        WindowFrameBound::UnboundedPreceding => 0,
        WindowFrameBound::Preceding(offset) => position - *offset as i64,
        WindowFrameBound::CurrentRow => position,
        WindowFrameBound::Following(offset) => position + *offset as i64,
        WindowFrameBound::UnboundedFollowing => len as i64 - 1,
    }
}

/// Aggregate one frame with the same rules as the plain aggregate path.
///
/// `SUM` keeps the numeric family it was given, `AVG` is always `F64`, `COUNT`
/// is `I64`, and `MIN` / `MAX` keep the type of the value they picked. An empty
/// frame therefore yields `0` for `COUNT` and `NULL` for the rest.
fn eval_window_aggregate(
    field: &WindowField,
    frame_rows: &[QueryRow],
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    outer: Option<&OuterRow<'_>>,
) -> Result<Value> {
    let argument = || {
        field
            .argument
            .clone()
            .ok_or_else(|| query_runtime_error(format!("`{}` requires an argument expression", field.function.as_str())))
    };
    let name = String::new();
    let aggregate = match field.function {
        WindowFunction::Sum => AggregateField::Sum { name, expr: argument()? },
        WindowFunction::Avg => AggregateField::Avg { name, expr: argument()? },
        WindowFunction::Count => AggregateField::Count { name, expr: field.argument.clone() },
        WindowFunction::Min | WindowFunction::Max => {
            return window_extreme(field, argument()?, frame_rows, contract, records, bound_values, outer);
        }
        WindowFunction::RowNumber | WindowFunction::Rank | WindowFunction::DenseRank => {
            unreachable!("ranking functions never read the frame")
        }
    };
    let (_, value) = eval_aggregate(&aggregate, frame_rows, contract, records, bound_values, outer)?;
    Ok(value)
}

/// `MIN(expr)` / `MAX(expr)` over one frame, keeping the picked value's type.
///
/// `NULL` values are skipped, so a frame of only `NULL`s is `NULL`. A tie keeps
/// the first value it saw, which preserves that value's exact numeric kind.
fn window_extreme(
    field: &WindowField,
    expr: ProjectExpr,
    frame_rows: &[QueryRow],
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
    outer: Option<&OuterRow<'_>>,
) -> Result<Value> {
    let mut best: Option<Value> = None;
    for row in frame_rows {
        let value = eval_project_expr(&expr, row, contract, records, bound_values, outer)?;
        if matches!(value, Value::Null) {
            continue;
        }
        best = Some(match best {
            None => value,
            Some(current) => {
                let ord = compare_values(&value, &current);
                let replace = match field.function {
                    WindowFunction::Min => ord == std::cmp::Ordering::Less,
                    _ => ord == std::cmp::Ordering::Greater,
                };
                if replace { value } else { current }
            }
        });
    }
    Ok(best.unwrap_or(Value::Null))
}

/// Build a canonical string key for a GROUP BY value vector.
///
/// `Value` does not implement `Hash`/`Eq`, so we serialize each value into a
/// stable text form and join with a separator that cannot appear inside a
/// serialized value. This is correct because YYDB group keys are primitive
/// scalars — never row or blob values.
fn group_key_to_string(values: &[Value]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(values.len() * 8);
    for (i, value) in values.iter().enumerate() {
        if i > 0 {
            out.push_str("\x1f"); // Unit separator — cannot appear in user data
        }
        match value {
            Value::Null => out.push_str("null"),
            Value::Bool(b) => write!(out, "bool:{b}").unwrap(),
            Value::I64(n) => write!(out, "i64:{n}").unwrap(),
            Value::U64(n) => write!(out, "u64:{n}").unwrap(),
            Value::F64(n) => write!(out, "f64:{n}").unwrap(),
            Value::Text(s) => {
                out.push_str("text:");
                out.push_str(s.as_str());
            }
            Value::Bytes(b) => write!(out, "bytes:<{}>", b.len()).unwrap(),
            Value::Uuid(u) => write!(out, "uuid:{u}").unwrap(),
            _ => write!(out, "other:{value:?}").unwrap(),
        }
    }
    out
}
