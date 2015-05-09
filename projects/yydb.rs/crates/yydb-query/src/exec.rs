//! Execute Phase 1 query ops against stored table rows.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use vos::types::ResolvedContract;
use yydb_types::{Error, Result, Value};

use super::{
    catalog::table_row_key_name,
    lower,
    ops::{
        AggregateField, BinaryOp, BoundValue, CmpOp, Expr, JoinKind, Pred, ProjectExpr, ProjectField, QueryOp, QueryRow,
        SetOpKind,
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

/// Execute a pre-lowered op list. `bound_values` supplies parameters for
/// `BoundValue::Slot { index }` entries; pass `&[]` when all values are
/// `BoundValue::Literal`.
pub fn execute_ops(
    ops: &[QueryOp],
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
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
                let predicate = resolve_subquery_predicates(predicate, contract, records, bound_values)?;
                rows.retain(|row| eval_pred(&predicate, row, table, contract, records, bound_values));
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
                rows = execute_join(*kind, &rows, &right_rows, &prefix, on, contract, records, bound_values)?;
            }
            QueryOp::JoinSubquery { kind, right, right_alias, on } => {
                if current_table.is_none() {
                    return Err(Error::Schema { message: "join without a preceding table scan".into() });
                }
                let right_rows = execute_ops(right, contract, records, bound_values)?;
                rows = execute_join(*kind, &rows, &right_rows, right_alias, on, contract, records, bound_values)?;
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
                )?;
            }
            QueryOp::Aggregate { group_keys, aggregates } => {
                rows = execute_aggregate(&rows, group_keys, aggregates, contract, records, bound_values)?;
            }
            QueryOp::Project { fields } => {
                let table = current_table
                    .as_deref()
                    .ok_or_else(|| Error::Schema { message: "project without a preceding table scan".into() })?;
                // A scalar subquery does not read the row being projected, so it
                // is resolved once for the whole pipeline instead of once per row.
                let fields = resolve_projection_subqueries(fields, contract, records, bound_values)?;
                rows = rows
                    .into_iter()
                    .map(|row| project_row(&fields, &row, table, contract, records, bound_values))
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
    let mut out = QueryRow::new();
    for field in fields {
        match &field.expr {
            ProjectExpr::Scalar { path } => {
                let value = read_path(path, row, table, contract, records);
                out.insert(field.name.clone(), value);
            }
            ProjectExpr::Nested { path, fields: nested } => {
                let nested_value = match contract {
                    Some(contract) => {
                        let resolved = resolve::resolve_row_at_path(path, row, table, contract, records);
                        match resolved {
                            Ok((nested_row, nested_table)) => {
                                let nested_projected =
                                    project_row(nested, &nested_row, &nested_table, Some(contract), records, bound_values)?;
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
                let value = eval_expr(inner, row, table, contract, records, bound_values)?;
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
/// against. Neither subquery can read the row under test, so running each one
/// once per filter or join keeps evaluation cheap and surfaces subquery errors,
/// which a per-row evaluation would have to swallow.
pub(crate) fn resolve_subquery_predicates(
    predicate: &Pred,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
) -> Result<Pred> {
    let resolve = |inner: &Pred| resolve_subquery_predicates(inner, contract, records, bound_values);
    Ok(match predicate {
        Pred::Exists { ops, negated } => {
            let found = !execute_ops(ops, contract, records, bound_values)?.is_empty();
            if found == *negated { Pred::False } else { Pred::True }
        }
        Pred::InSubquery { path, ops, negated } => {
            let rows = execute_ops(ops, contract, records, bound_values)?;
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
        Pred::FieldCmpSubquery { path, op, ops } => Pred::FieldCmpScalar {
            path: path.clone(),
            op: *op,
            value: scalar_subquery_value(ops, contract, records, bound_values)?,
        },
        Pred::And(left, right) => Pred::And(Box::new(resolve(left)?), Box::new(resolve(right)?)),
        Pred::Or(left, right) => Pred::Or(Box::new(resolve(left)?), Box::new(resolve(right)?)),
        Pred::Not(inner) => Pred::Not(Box::new(resolve(inner)?)),
        other => other.clone(),
    })
}

pub(crate) fn eval_pred(
    pred: &Pred,
    row: &QueryRow,
    table: &str,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
) -> bool {
    match pred {
        Pred::True => true,
        Pred::False => false,
        Pred::FieldBool { path, value } => {
            let left = read_path(path, row, table, contract, records);
            let right = match resolve_bound_value(value, bound_values) {
                Ok(v) => v,
                Err(_) => return false, // unresolved slot → predicate fails safe
            };
            matches!((&left, &right), (Value::Bool(a), Value::Bool(b)) if a == b)
        }
        Pred::FieldCmp { path, op, value } => {
            let left = read_path(path, row, table, contract, records);
            let right = match resolve_bound_value(value, bound_values) {
                Ok(v) => v,
                Err(_) => return false,
            };
            cmp_values(&left, *op, &right)
        }
        Pred::FieldCmpField { left_path, op, right_path } => {
            let left = read_path(left_path, row, table, contract, records);
            let right = read_path(right_path, row, table, contract, records);
            cmp_values(&left, *op, &right)
        }
        Pred::And(left, right) => {
            eval_pred(left, row, table, contract, records, bound_values)
                && eval_pred(right, row, table, contract, records, bound_values)
        }
        Pred::Or(left, right) => {
            eval_pred(left, row, table, contract, records, bound_values)
                || eval_pred(right, row, table, contract, records, bound_values)
        }
        Pred::Not(inner) => !eval_pred(inner, row, table, contract, records, bound_values),
        Pred::InSet { path, values, negated } => {
            let left = read_path(path, row, table, contract, records);
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
            let left = read_path(path, row, table, contract, records);
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
) -> Result<Value> {
    let rows = execute_ops(ops, contract, records, bound_values)?;
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
) -> Value {
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

fn compare_values(left: &Value, right: &Value) -> std::cmp::Ordering {
    match (left, right) {
        (Value::Null, Value::Null) => std::cmp::Ordering::Equal,
        (Value::Null, _) => std::cmp::Ordering::Less,
        (_, Value::Null) => std::cmp::Ordering::Greater,
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        (Value::I64(a), Value::I64(b)) => a.cmp(b),
        (Value::Text(a), Value::Text(b)) => a.cmp(b),
        (Value::Uuid(a), Value::Uuid(b)) => a.cmp(b),
        (Value::Text(a), Value::Uuid(b)) => a.as_str().cmp(&b.to_string()),
        (Value::Uuid(a), Value::Text(b)) => a.to_string().cmp(b),
        _ => std::cmp::Ordering::Equal,
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
) -> Result<Value> {
    match expr {
        Expr::Column { path } => Ok(read_path(path, row, table, contract, records)),
        Expr::Bound(bv) => resolve_bound_value(bv, bound_values),
        Expr::Case { branches, else_branch } => {
            eval_case(branches, else_branch, row, table, contract, records, bound_values)
        }
        Expr::Cast { expr: inner, target } => {
            let value = eval_expr(inner, row, table, contract, records, bound_values)?;
            eval_cast(&value, target)
        }
        Expr::Binary { op, left, right } => {
            let l = eval_expr(left, row, table, contract, records, bound_values)?;
            let r = eval_expr(right, row, table, contract, records, bound_values)?;
            eval_binary(*op, &l, &r)
        }
        Expr::Not(inner) => {
            let val = eval_expr(inner, row, table, contract, records, bound_values)?;
            Ok(Value::Bool(!is_truthy(&val)))
        }
        Expr::Subquery { ops } => scalar_subquery_value(ops, contract, records, bound_values),
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

/// Replace every [`Expr::Subquery`] of an expression with the literal value it
/// produced.
fn resolve_scalar_subqueries(
    expr: &Expr,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
) -> Result<Expr> {
    let resolve = |inner: &Expr| resolve_scalar_subqueries(inner, contract, records, bound_values);
    Ok(match expr {
        Expr::Subquery { ops } => {
            Expr::Bound(BoundValue::Literal(scalar_subquery_value(ops, contract, records, bound_values)?))
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
) -> Result<Value> {
    for (when, then) in branches {
        let when_val = eval_expr(when, row, table, contract, records, bound_values)?;
        if is_truthy(&when_val) {
            return eval_expr(then, row, table, contract, records, bound_values);
        }
    }
    match else_branch {
        Some(els) => eval_expr(els, row, table, contract, records, bound_values),
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

/// Evaluate a binary expression (comparison or logical) for CASE WHEN
/// predicates and nested scalar comparisons. NULL propagation follows SQL 3VL
/// semantics for comparisons (NULL when either operand is NULL) and treats
/// NULL as falsy for AND/OR.
fn eval_binary(op: BinaryOp, left: &Value, right: &Value) -> Result<Value> {
    use BinaryOp::*;
    // AND / OR use SQL truth tables with NULL propagation.
    match op {
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
                And | Or => unreachable!(),
            };
            Ok(Value::Bool(result))
        }
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
) -> Result<Vec<QueryRow>> {
    // A subquery in `ON` is independent of both rows, so it is resolved once for
    // the whole join instead of once per pair.
    let on = resolve_subquery_predicates(on, contract, records, bound_values)?;
    let on = &on;
    match kind {
        JoinKind::Inner => execute_inner_join(left_rows, right_rows, prefix, on, contract, records, bound_values),
        JoinKind::Left => execute_left_join(left_rows, right_rows, prefix, on, contract, records, bound_values),
        JoinKind::Right => execute_right_join(left_rows, right_rows, prefix, on, contract, records, bound_values),
        JoinKind::Full => execute_full_join(left_rows, right_rows, prefix, on, contract, records, bound_values),
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
            if eval_pred(on, &merged, "", contract, records, bound_values) {
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
            if eval_pred(on, &merged, "", contract, records, bound_values) {
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
            if eval_pred(on, &merged, "", contract, records, bound_values) {
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
            if eval_pred(on, &merged, "", contract, records, bound_values) {
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
) -> Result<Vec<QueryRow>> {
    let left_rows = execute_ops(left_ops, contract, records, bound_values)?;
    let right_rows = execute_ops(right_ops, contract, records, bound_values)?;

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
) -> Result<Vec<QueryRow>> {
    if group_keys.is_empty() {
        // Global aggregate: all rows form one group.
        let mut out = QueryRow::new();
        for agg in aggregates {
            let (name, value) = eval_aggregate(agg, rows, contract, records, bound_values)?;
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
                ProjectExpr::Scalar { path } => read_path(path, row, "", contract, records),
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
                    ProjectExpr::Scalar { path } => read_path(path, first_row, "", contract, records),
                    ProjectExpr::Nested { .. } => Value::Null,
                    ProjectExpr::Computed(_) => Value::Null,
                };
                out.insert(gk.name.clone(), value);
            }
        }
        // Aggregate values.
        let group_row_vec: Vec<QueryRow> = group_rows.iter().cloned().cloned().collect();
        for agg in aggregates {
            let (name, value) = eval_aggregate(agg, &group_row_vec, contract, records, bound_values)?;
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
) -> Result<(String, Value)> {
    match agg {
        AggregateField::Count { name, expr: None } => {
            // COUNT(*) — number of rows in the group.
            Ok((name.clone(), Value::I64(group_rows.len() as i64)))
        }
        AggregateField::Count { name, expr: Some(inner) } => {
            // COUNT(expr) — non-null values of the expression.
            let count = group_rows
                .iter()
                .filter(|row| eval_project_expr(inner, row, contract, records, bound_values) != Value::Null)
                .count();
            Ok((name.clone(), Value::I64(count as i64)))
        }
        AggregateField::CountDistinct { name, expr } => {
            // COUNT(DISTINCT expr) — distinct non-null values of the expression.
            // Value implements neither Hash nor Ord so we serialise each
            // non-null value to its Debug form for set membership. MVP scalar
            // values (Null, Bool, I64, Text, Uuid …) are all uniquely
            // represented by their Debug output.
            let mut seen: BTreeSet<String> = BTreeSet::new();
            for row in group_rows {
                let value = eval_project_expr(expr, row, contract, records, bound_values);
                if !matches!(value, Value::Null) {
                    seen.insert(format!("{value:?}"));
                }
            }
            Ok((name.clone(), Value::I64(seen.len() as i64)))
        }
        AggregateField::Sum { name, expr } => {
            let mut sum: i64 = 0;
            let mut saw_any = false;
            for row in group_rows {
                let value = eval_project_expr(expr, row, contract, records, bound_values);
                match value {
                    Value::Null => {}
                    Value::I64(n) => {
                        sum = sum.wrapping_add(n);
                        saw_any = true;
                    }
                    Value::U64(n) => {
                        // Clamp to i64 range conservatively.
                        sum = sum.wrapping_add(n.min(i64::MAX as u64) as i64);
                        saw_any = true;
                    }
                    _ => {}
                }
            }
            let result = if saw_any { Value::I64(sum) } else { Value::Null };
            Ok((name.clone(), result))
        }
        AggregateField::Avg { name, expr } => {
            let mut sum: f64 = 0.0;
            let mut count: u64 = 0;
            for row in group_rows {
                let value = eval_project_expr(expr, row, contract, records, bound_values);
                match value {
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
                    _ => {}
                }
            }
            let result = if count > 0 { Value::F64(sum / count as f64) } else { Value::Null };
            Ok((name.clone(), result))
        }
    }
}

/// Evaluate a `ProjectExpr` against a single row (used inside aggregate
/// evaluation where we have no table context).
fn eval_project_expr(
    expr: &ProjectExpr,
    row: &QueryRow,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
) -> Value {
    match expr {
        ProjectExpr::Scalar { path } => read_path(path, row, "", contract, records),
        ProjectExpr::Nested { path, .. } => read_path(path, row, "", contract, records),
        ProjectExpr::Computed(inner) => eval_expr(inner, row, "", contract, records, bound_values).unwrap_or(Value::Null),
    }
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
