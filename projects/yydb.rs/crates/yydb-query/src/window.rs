//! Record-key windows for narrowed reads.
//!
//! A read plan that scans one table and filters on that table's primary key can
//! name a contiguous range of record keys. Storage hosts use that range to read
//! only the rows the plan can possibly keep, instead of materialising every row
//! of the database for each statement.
//!
//! The window is a *superset* of the rows the predicate keeps. The filter still
//! runs unchanged on top of it, so a missing or wrong window can only widen the
//! read, never change a result. Anything this module cannot prove — joins,
//! subqueries, a second table, non-text range bounds — returns `None` and the
//! caller falls back to a full read.

use vos::types::{CanonicalScalar, ResolvedCanonicalType, ResolvedContract, ResolvedTypeContract};
use yydb_types::{Error, Result, Value};

use super::{
    catalog::{primary_field_name, table_row_key_name, table_type_by_name},
    constraints,
    ops::{BoundValue, CmpOp, Expr, Pred, QueryOp},
    store,
};

/// Half-open window over the record keys of one table.
///
/// `start` and `end` are physical record names (`__yydb/v0/row/<table>/<pk>`),
/// so they order exactly the way the record tree stores them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadWindow {
    /// Inclusive lower record name.
    pub start: String,
    /// Exclusive upper record name. `None` reads to the last key.
    pub end: Option<String>,
}

/// Derive the record window a read plan can be confined to.
///
/// Returns `None` when the plan can read more than a single-table primary-key
/// range covers, or when the bounds are not expressible as a lexicographic
/// range (for example a numeric range on an integer key, whose text order is
/// not numeric).
pub fn read_window(ops: &[QueryOp], contract: &ResolvedContract, bound_values: &[Value]) -> Result<Option<ReadWindow>> {
    let Some(QueryOp::Scan { table }) = ops.first()
    else {
        return Ok(None);
    };
    if ops.iter().skip(1).any(reads_other_data) {
        return Ok(None);
    }
    let mut scans = 0_usize;
    let mut filters: Vec<&Pred> = Vec::new();
    for op in ops {
        match op {
            QueryOp::Scan { .. } => scans += 1,
            QueryOp::Filter { predicate } => filters.push(predicate),
            _ => {}
        }
    }
    if scans != 1 || filters.is_empty() || filters.iter().any(|pred| pred_has_subquery(pred)) {
        return Ok(None);
    }

    let entry = table_type_by_name(contract, table)?;
    let pk_field = primary_field_name(entry)?;
    let storage_table = table_row_key_name(contract, table)?;
    let pk_is_text = text_primary_field(entry, pk_field);

    let mut start: Option<String> = None;
    let mut end: Option<String> = None;
    for filter in filters {
        collect_bounds(filter, pk_field, pk_is_text, &storage_table, bound_values, &mut start, &mut end)?;
    }
    if start.is_none() && end.is_none() {
        return Ok(None);
    }
    Ok(Some(ReadWindow { start: start.unwrap_or_else(|| store::row_key(&storage_table, "")), end }))
}

/// Any operator that reads rows outside the leading scan.
fn reads_other_data(op: &QueryOp) -> bool {
    matches!(op, QueryOp::Scan { .. } | QueryOp::Join { .. } | QueryOp::JoinSubquery { .. } | QueryOp::SetOperation { .. })
}

/// Whether a predicate runs a nested pipeline.
fn pred_has_subquery(pred: &Pred) -> bool {
    match pred {
        Pred::True | Pred::False | Pred::FieldBool { .. } | Pred::FieldCmp { .. } | Pred::TextTest { .. } => false,
        Pred::FieldCmpField { .. } | Pred::InSet { .. } | Pred::FieldCmpScalar { .. } | Pred::ExprInSet { .. } => false,
        Pred::Exists { .. } | Pred::InSubquery { .. } | Pred::FieldCmpSubquery { .. } | Pred::SubqueryAsBool { .. } => true,
        Pred::ExprInSubquery { .. } => true,
        Pred::And(left, right) | Pred::Or(left, right) => pred_has_subquery(left) || pred_has_subquery(right),
        Pred::Not(inner) => pred_has_subquery(inner),
        Pred::ExprCmp { expr, .. }
        | Pred::ExprIsNull { expr, .. }
        | Pred::ExprIsTruth { expr, .. }
        | Pred::ExprInList { expr, .. }
        | Pred::ExprBetween { expr, .. } => expr_has_subquery(expr),
    }
}

fn expr_has_subquery(expr: &Expr) -> bool {
    match expr {
        Expr::Subquery { .. } => true,
        Expr::Column { .. } | Expr::Bound(_) => false,
        Expr::Case { branches, else_branch } => {
            branches.iter().any(|(when, then)| expr_has_subquery(when) || expr_has_subquery(then))
                || else_branch.as_deref().is_some_and(expr_has_subquery)
        }
        Expr::Cast { expr, .. } => expr_has_subquery(expr),
        Expr::Binary { left, right, .. } => expr_has_subquery(left) || expr_has_subquery(right),
        Expr::Not(inner) => expr_has_subquery(inner),
        Expr::Function { args, .. } => args.iter().any(expr_has_subquery),
    }
}

/// Whether the primary key is UTF-8 text, the only type whose stored key order
/// is the same lexicographic order the filter compares against.
fn text_primary_field(entry: &ResolvedTypeContract, pk_field: &str) -> bool {
    entry
        .fields
        .iter()
        .find(|field| field.canonical_name == pk_field)
        .is_some_and(|field| matches!(field.canonical_type, ResolvedCanonicalType::Scalar(CanonicalScalar::Utf8)))
}

/// Intersect a predicate's primary-key comparisons into the window bounds.
fn collect_bounds(
    pred: &Pred,
    pk_field: &str,
    pk_is_text: bool,
    storage_table: &str,
    bound_values: &[Value],
    start: &mut Option<String>,
    end: &mut Option<String>,
) -> Result<()> {
    match pred {
        Pred::And(left, right) => {
            collect_bounds(left, pk_field, pk_is_text, storage_table, bound_values, start, end)?;
            collect_bounds(right, pk_field, pk_is_text, storage_table, bound_values, start, end)?;
        }
        Pred::ExprCmp { expr, op, value } => {
            if let Expr::Column { path } = expr.as_ref()
                && path.len() == 1
                && path[0] == pk_field
            {
                apply_comparison(*op, value, pk_is_text, storage_table, bound_values, start, end)?;
            }
        }
        Pred::FieldCmp { path, op, value } => {
            if path.len() == 1 && path[0] == pk_field {
                apply_comparison(*op, value, pk_is_text, storage_table, bound_values, start, end)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn apply_comparison(
    op: CmpOp,
    value: &BoundValue,
    pk_is_text: bool,
    storage_table: &str,
    bound_values: &[Value],
    start: &mut Option<String>,
    end: &mut Option<String>,
) -> Result<()> {
    let value = resolve_bound(value, bound_values)?;
    // Only a UTF-8 text key orders lexicographically the way the filter
    // compares, so a range bound maps straight onto a record-key range. For
    // every other key type only equality still names one exact key.
    let text = match (&value, op) {
        (Value::Text(text), _) if pk_is_text => text.clone(),
        (value, CmpOp::Eq) => constraints::value_text(value)?,
        _ => return Ok(()),
    };
    let key = store::row_key(storage_table, &text);
    match op {
        CmpOp::Eq => {
            lift_start(start, key.clone());
            lower_end(end, format!("{key}\u{0}"));
        }
        CmpOp::Ge => lift_start(start, key),
        CmpOp::Gt => lift_start(start, format!("{key}\u{0}")),
        CmpOp::Le => lower_end(end, format!("{key}\u{0}")),
        CmpOp::Lt => lower_end(end, key),
        CmpOp::Ne => {}
    }
    Ok(())
}

fn resolve_bound(value: &BoundValue, bound_values: &[Value]) -> Result<Value> {
    match value {
        BoundValue::Literal(value) => Ok(value.clone()),
        BoundValue::Slot { index, .. } => bound_values
            .get(*index as usize)
            .cloned()
            .ok_or_else(|| Error::Schema { message: format!("parameter slot {index} has no bound value") }),
    }
}

fn lift_start(slot: &mut Option<String>, candidate: String) {
    *slot = Some(match slot.take() {
        Some(current) if current >= candidate => current,
        _ => candidate,
    });
}

fn lower_end(slot: &mut Option<String>, candidate: String) {
    *slot = Some(match slot.take() {
        Some(current) if current <= candidate => current,
        _ => candidate,
    });
}
