//! Execute Phase 1 query ops against stored table rows.

use std::collections::BTreeMap;

use vos::ast::catalog::CatalogSnapshot;
use yydb_types::{Error, Result, Value};

use super::lower;
use super::ops::{CmpOp, LiteralKind, Pred, ProjectExpr, ProjectField, QueryOp, QueryRow};
use super::resolve;
use super::store;

pub fn execute(
    source: &str,
    catalog: Option<&CatalogSnapshot>,
    records: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<QueryRow>> {
    let program = vos::parse_program(source).map_err(|diagnostics| Error::Schema {
        message: diagnostics
            .errors
            .first()
            .map(|diag| diag.message.clone())
            .unwrap_or_else(|| "VOS query parse failed".into()),
    })?;
    let ops = lower::lower_program(&program)?;
    execute_ops(&ops, catalog, records)
}

pub fn execute_ops(
    ops: &[QueryOp],
    catalog: Option<&CatalogSnapshot>,
    records: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<QueryRow>> {
    let mut rows: Vec<QueryRow> = Vec::new();
    let mut current_table: Option<String> = None;
    for op in ops {
        match op {
            QueryOp::Scan { table } => {
                current_table = Some(table.clone());
                rows = store::load_table(records, table)?;
            }
            QueryOp::Filter { predicate } => {
                let table = current_table.as_deref().ok_or_else(|| Error::Schema {
                    message: "filter without a preceding table scan".into(),
                })?;
                rows.retain(|row| eval_pred(predicate, row, table, catalog, records));
            }
            QueryOp::Project { fields } => {
                let table = current_table.as_deref().ok_or_else(|| Error::Schema {
                    message: "project without a preceding table scan".into(),
                })?;
                rows = rows
                    .into_iter()
                    .map(|row| project_row(fields, &row, table, catalog, records))
                    .collect();
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
                let skip = (*count as usize).min(rows.len());
                rows = rows.into_iter().skip(skip).collect();
            }
            QueryOp::Take { count } => {
                rows.truncate(*count as usize);
            }
            QueryOp::Collect => {}
        }
    }
    Ok(rows)
}

fn project_row(
    fields: &[ProjectField],
    row: &QueryRow,
    table: &str,
    catalog: Option<&CatalogSnapshot>,
    records: &BTreeMap<String, Vec<u8>>,
) -> QueryRow {
    let mut out = QueryRow::new();
    for field in fields {
        match &field.expr {
            ProjectExpr::Scalar { path } => {
                let value = read_path(path, row, table, catalog, records);
                out.insert(field.name.clone(), value);
            }
            ProjectExpr::Nested { path, fields: nested } => {
                let nested_value = match catalog {
                    Some(catalog) => resolve::resolve_row_at_path(path, row, table, catalog, records)
                        .map(|(nested_row, nested_table)| {
                            Value::Row(project_row(
                                nested,
                                &nested_row,
                                &nested_table,
                                Some(catalog),
                                records,
                            ))
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
    catalog: Option<&CatalogSnapshot>,
    records: &BTreeMap<String, Vec<u8>>,
) -> bool {
    match pred {
        Pred::True => true,
        Pred::False => false,
        Pred::FieldBool { path, value } => {
            let left = read_path(path, row, table, catalog, records);
            matches!(left, Value::Bool(b) if b == *value)
        }
        Pred::FieldCmp {
            path,
            op,
            literal,
            kind,
        } => {
            let left = read_path(path, row, table, catalog, records);
            let right = decode_literal(literal, *kind);
            cmp_values(&left, *op, &right)
        }
        Pred::And(left, right) => {
            eval_pred(left, row, table, catalog, records)
                && eval_pred(right, row, table, catalog, records)
        }
        Pred::Or(left, right) => {
            eval_pred(left, row, table, catalog, records)
                || eval_pred(right, row, table, catalog, records)
        }
    }
}

fn read_path(
    path: &[String],
    row: &QueryRow,
    table: &str,
    catalog: Option<&CatalogSnapshot>,
    records: &BTreeMap<String, Vec<u8>>,
) -> Value {
    if path.len() == 1 {
        return row.get(&path[0]).cloned().unwrap_or(Value::Null);
    }
    if let Some(catalog) = catalog {
        return resolve::resolve_field_path(path, row, table, catalog, records)
            .unwrap_or(Value::Null);
    }
    Value::Null
}

fn decode_literal(text: &str, kind: LiteralKind) -> Value {
    match kind {
        LiteralKind::Null => Value::Null,
        LiteralKind::Bool => Value::Bool(text == "true"),
        LiteralKind::Int => Value::I64(text.parse().unwrap_or(0)),
        LiteralKind::Str => Value::Text(text.to_string()),
    }
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
