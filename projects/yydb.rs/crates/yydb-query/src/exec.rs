//! Execute Phase 1 query ops against stored table rows.

use std::collections::{BTreeMap, HashMap};

use vos::types::ResolvedContract;
use yydb_types::{Error, Result, Value};

use super::{
    catalog::table_row_key_name,
    lower,
    ops::{AggregateField, BoundValue, CmpOp, JoinKind, Pred, ProjectExpr, ProjectField, QueryOp, QueryRow},
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
            QueryOp::Filter { predicate } => {
                let table = current_table
                    .as_deref()
                    .ok_or_else(|| Error::Schema { message: "filter without a preceding table scan".into() })?;
                rows.retain(|row| eval_pred(predicate, row, table, contract, records, bound_values));
            }
            QueryOp::Join { kind, right, right_alias, on } => {
                let table = current_table
                    .as_deref()
                    .ok_or_else(|| Error::Schema { message: "join without a preceding table scan".into() })?;
                match kind {
                    JoinKind::Inner => {
                        rows = execute_inner_join(
                            &rows,
                            table,
                            right,
                            right_alias.as_deref(),
                            on,
                            contract,
                            records,
                            bound_values,
                        )?;
                    }
                }
            }
            QueryOp::Aggregate { group_keys, aggregates } => {
                rows = execute_aggregate(&rows, group_keys, aggregates, contract, records, bound_values)?;
            }
            QueryOp::Project { fields } => {
                let table = current_table
                    .as_deref()
                    .ok_or_else(|| Error::Schema { message: "project without a preceding table scan".into() })?;
                rows = rows.into_iter().map(|row| project_row(fields, &row, table, contract, records)).collect();
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

fn project_row(
    fields: &[ProjectField],
    row: &QueryRow,
    table: &str,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
) -> QueryRow {
    let mut out = QueryRow::new();
    for field in fields {
        match &field.expr {
            ProjectExpr::Scalar { path } => {
                let value = read_path(path, row, table, contract, records);
                out.insert(field.name.clone(), value);
            }
            ProjectExpr::Nested { path, fields: nested } => {
                let nested_value = match contract {
                    Some(contract) => resolve::resolve_row_at_path(path, row, table, contract, records)
                        .map(|(nested_row, nested_table)| {
                            Value::Row(project_row(nested, &nested_row, &nested_table, Some(contract), records))
                        })
                        .unwrap_or(Value::Null),
                    None => Value::Null,
                };
                out.insert(field.name.clone(), nested_value);
            }
        }
    }
    out
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
    }
}

fn read_path(
    path: &[String],
    row: &QueryRow,
    table: &str,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
) -> Value {
    if path.len() == 1 {
        return row.get(&path[0]).cloned().unwrap_or(Value::Null);
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

// ─── join ────────────────────────────────────────────────────────────────

fn execute_inner_join(
    left_rows: &[QueryRow],
    _left_table: &str,
    right: &str,
    right_alias: Option<&str>,
    on: &Pred,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
    bound_values: &[Value],
) -> Result<Vec<QueryRow>> {
    // Load right table rows.
    let storage_right = match contract {
        Some(contract) => table_row_key_name(contract, right)?,
        None => right.to_string(),
    };
    let right_rows = store::load_table(records, &storage_right)?;

    // Column prefix for the right table in the joined row.
    let prefix = match right_alias {
        Some(a) => a.to_string(),
        None => right.to_string(),
    };

    // Nested loop: for each left row, for each right row, check ON condition.
    // The ON predicate operates on a merged row where both sides are
    // addressable via their table-qualified column names (prefix.column).
    let mut out: Vec<QueryRow> = Vec::with_capacity(left_rows.len().min(right_rows.len()));
    for left_row in left_rows {
        for right_row in &right_rows {
            let merged = merge_rows(left_row, right_row, &prefix);
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
    _bound_values: &[Value],
) -> Value {
    match expr {
        ProjectExpr::Scalar { path } => read_path(path, row, "", contract, records),
        ProjectExpr::Nested { path, .. } => read_path(path, row, "", contract, records),
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
