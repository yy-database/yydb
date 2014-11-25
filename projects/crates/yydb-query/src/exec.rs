//! Execute Phase 1 query ops against stored table rows.

use std::collections::BTreeMap;

use vos::types::ResolvedContract;
use yydb_types::{Error, Result, Value};

use super::{
    catalog::table_row_key_name,
    lower,
    ops::{BoundValue, CmpOp, Pred, ProjectExpr, ProjectField, QueryOp, QueryRow},
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

fn eval_pred(
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
        Pred::And(left, right) => {
            eval_pred(left, row, table, contract, records, bound_values)
                && eval_pred(right, row, table, contract, records, bound_values)
        }
        Pred::Or(left, right) => {
            eval_pred(left, row, table, contract, records, bound_values)
                || eval_pred(right, row, table, contract, records, bound_values)
        }
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
