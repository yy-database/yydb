//! Execute Phase 1 query ops against stored table rows.

use std::collections::BTreeMap;

use yydb_types::{Error, Result, Value};

use super::lower;
use super::ops::{CmpOp, LiteralKind, Pred, QueryOp, QueryRow};
use super::store;

pub fn execute(source: &str, records: &BTreeMap<String, Vec<u8>>) -> Result<Vec<QueryRow>> {
    let program = vos::parse_program(source).map_err(|diagnostics| Error::Schema {
        message: diagnostics
            .errors
            .first()
            .map(|diag| diag.message.clone())
            .unwrap_or_else(|| "VOS query parse failed".into()),
    })?;
    let ops = lower::lower_program(&program)?;
    execute_ops(&ops, records)
}

pub fn execute_ops(ops: &[QueryOp], records: &BTreeMap<String, Vec<u8>>) -> Result<Vec<QueryRow>> {
    let mut rows: Vec<QueryRow> = Vec::new();
    for op in ops {
        match op {
            QueryOp::Scan { table } => {
                rows = store::load_table(records, table)?;
            }
            QueryOp::Filter { predicate } => {
                rows.retain(|row| eval_pred(predicate, row));
            }
            QueryOp::Project { fields } => {
                rows = rows
                    .into_iter()
                    .map(|row| {
                        let mut out = QueryRow::new();
                        for field in fields {
                            let src = field.from.as_deref().unwrap_or(field.name.as_str());
                            if let Some(value) = row.get(src) {
                                out.insert(field.name.clone(), value.clone());
                            }
                        }
                        out
                    })
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

fn eval_pred(pred: &Pred, row: &QueryRow) -> bool {
    match pred {
        Pred::True => true,
        Pred::False => false,
        Pred::FieldBool { field, value } => matches!(row.get(field), Some(Value::Bool(b)) if b == value),
        Pred::FieldCmp {
            field,
            op,
            literal,
            kind,
        } => {
            let left = row.get(field).cloned().unwrap_or(Value::Null);
            let right = decode_literal(literal, *kind);
            cmp_values(&left, *op, &right)
        }
        Pred::And(left, right) => eval_pred(left, row) && eval_pred(right, row),
        Pred::Or(left, right) => eval_pred(left, row) || eval_pred(right, row),
    }
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
