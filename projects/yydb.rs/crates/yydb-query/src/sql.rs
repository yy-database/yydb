//! SQL frontend: lower an Oak SQL AST into the shared physical operators.
//!
//! Parsing belongs to `oak-sql`; this module only *lowers*. It resolves no
//! catalog information and touches no storage, so the same plan can be prepared
//! once and executed against any schema that declares the referenced tables.

use oak_sql::ast::{
    BinaryOperator, DeleteStatement, Expression, GroupByClause, GroupByItem, Identifier, InsertStatement, JoinClause,
    JoinType, LimitClause, Literal, OrderByItem, SelectItem, SelectStatement, SqlRoot, SqlStatement, TableName,
    UnaryOperator, UpdateStatement,
};
use yydb_types::{Error, Result, Value};

use super::{
    ops::{
        AggregateField, BoundValue, CmpOp, JoinKind, Pred, ProjectExpr, ProjectField, QueryOp, SortKey, WriteAssignment,
        WriteBinOp, WriteExpr, WriteField, WriteOp,
    },
    params::{ParameterDescriptor, name_to_slot},
};

fn sql_error(message: impl Into<String>) -> Error {
    Error::Schema { message: message.into() }
}

/// One lowered SQL program: either a read pipeline or a write op list.
#[derive(Debug, Clone, PartialEq)]
pub struct SqlPlan {
    /// Read pipeline ending in [`QueryOp::Collect`]; empty for writes.
    pub read_ops: Vec<QueryOp>,
    /// Write operators; empty for reads.
    pub write_ops: Vec<WriteOp>,
    /// Tables read by this statement.
    pub read_set: Vec<String>,
    /// Tables written by this statement.
    pub write_set: Vec<String>,
}

/// Parse SQL source into the Oak SQL root node.
pub fn parse(source: &str) -> Result<SqlRoot> {
    oak_sql::parse(source).map_err(|diagnostics| sql_error(format!("SQL parse error: {diagnostics}")))
}

/// Lower a whole SQL source (exactly one statement) into shared operators.
pub fn lower(source: &str, parameters: &[ParameterDescriptor]) -> Result<SqlPlan> {
    let root = parse(source)?;
    lower_root(&root, parameters)
}

/// Lower an already parsed root node.
pub fn lower_root(root: &SqlRoot, parameters: &[ParameterDescriptor]) -> Result<SqlPlan> {
    let mut statements = root.statements.iter();
    let Some(statement) = statements.next()
    else {
        return Err(sql_error("SQL source contains no statement"));
    };
    if statements.next().is_some() {
        return Err(sql_error("SQL source must contain exactly one statement"));
    }
    match statement {
        SqlStatement::Select(select) => {
            let ops = lower_select(select, parameters)?;
            let table = select
                .from
                .as_ref()
                .map(|from| from.name.name.to_string())
                .ok_or_else(|| sql_error("SELECT requires a FROM clause"))?;
            Ok(SqlPlan { read_ops: ops, write_ops: Vec::new(), read_set: vec![table], write_set: Vec::new() })
        }
        SqlStatement::Insert(insert) => {
            let op = lower_insert(insert, parameters)?;
            Ok(SqlPlan {
                read_ops: Vec::new(),
                write_ops: vec![op],
                read_set: Vec::new(),
                write_set: vec![insert.table_name.name.name.to_string()],
            })
        }
        SqlStatement::Update(update) => {
            let op = lower_update(update, parameters)?;
            Ok(SqlPlan {
                read_ops: Vec::new(),
                write_ops: vec![op],
                read_set: Vec::new(),
                write_set: vec![update.table_name.name.name.to_string()],
            })
        }
        SqlStatement::Delete(delete) => {
            let op = lower_delete(delete, parameters)?;
            Ok(SqlPlan {
                read_ops: Vec::new(),
                write_ops: vec![op],
                read_set: Vec::new(),
                write_set: vec![delete.table_name.name.name.to_string()],
            })
        }
        SqlStatement::Error { message, .. } => Err(sql_error(format!("SQL statement error: {message}"))),
        SqlStatement::Create(_) | SqlStatement::Drop(_) | SqlStatement::Alter(_) => {
            Err(sql_error("DDL is owned by the schema document, not by SQL statements"))
        }
        SqlStatement::Transaction(_) | SqlStatement::SetNames(_) | SqlStatement::SetVariable(_) => {
            Err(sql_error("session statements are applied through the connection surface, not lowered into operators"))
        }
        SqlStatement::Show(_) | SqlStatement::Pragma(_) => {
            Err(sql_error("session introspection is not part of the YYDB SQL query surface"))
        }
        SqlStatement::Unknown { .. } => Err(sql_error("unknown SQL statement")),
    }
}

// ─── read pipeline ───────────────────────────────────────────────────────

fn lower_select(select: &SelectStatement, parameters: &[ParameterDescriptor]) -> Result<Vec<QueryOp>> {
    let table = select
        .from
        .as_ref()
        .map(|from| from.name.name.to_string())
        .ok_or_else(|| sql_error("SELECT requires a FROM clause"))?;

    let mut ops = vec![QueryOp::Scan { table }];

    // JOIN clauses.
    for join_clause in &select.joins {
        ops.push(lower_join_clause(join_clause, parameters)?);
    }

    // WHERE clause.
    if let Some(predicate) = &select.expr {
        ops.push(QueryOp::Filter { predicate: lower_predicate(predicate, parameters)? });
    }

    // Aggregate: GROUP BY + aggregate functions from SELECT items.
    let (group_keys, aggregates) = lower_aggregate(select, parameters)?;
    let has_aggregate = !aggregates.is_empty() || !group_keys.is_empty();
    if has_aggregate {
        ops.push(QueryOp::Aggregate { group_keys, aggregates });
    }

    // HAVING — MVP rejects until Aggregate + Filter composition lands.
    if select.having.is_some() {
        return Err(sql_error("HAVING clause is not yet supported by the YYDB query executor"));
    }

    // ORDER BY.
    if let Some(order_by) = &select.order_by {
        let mut keys = Vec::with_capacity(order_by.items.len());
        for item in &order_by.items {
            keys.push(lower_sort_key(item)?);
        }
        ops.push(QueryOp::Sort { keys });
    }

    // LIMIT.
    if let Some(limit) = &select.limit {
        lower_limit(limit, parameters, &mut ops)?;
    }

    // SELECT projection (without aggregate function items — those are
    // consumed by the Aggregate op above). When an Aggregate op was pushed
    // the row already carries every output column with its final name (GROUP
    // BY columns + aggregate aliases), so we skip Project entirely —
    // pushing Project with only the non-aggregate items would drop the
    // aggregate columns.
    if !has_aggregate {
        let fields = lower_projection(&select.items)?;
        if let Some(fields) = fields
            && !fields.is_empty()
        {
            ops.push(QueryOp::Project { fields });
        }
    }
    ops.push(QueryOp::Collect);
    Ok(ops)
}

// ─── join lowering ───────────────────────────────────────────────────────

fn lower_join_clause(join: &JoinClause, parameters: &[ParameterDescriptor]) -> Result<QueryOp> {
    let kind = match join.join_type {
        JoinType::Inner => JoinKind::Inner,
        JoinType::Left | JoinType::Right | JoinType::Full => {
            return Err(sql_error(format!(
                "join kind `{:?}` is not supported by the YYDB query executor (only INNER is MVP)",
                join.join_type
            )));
        }
    };
    let right = join.table.name.name.to_string();
    let on = join
        .on
        .as_ref()
        .map(|expr| lower_predicate(expr, parameters))
        .transpose()?
        .unwrap_or(Pred::True);
    Ok(QueryOp::Join { kind, right, right_alias: None, on })
}

// ─── aggregate lowering ──────────────────────────────────────────────────

/// Extract GROUP BY keys and aggregate function calls from a SELECT statement.
///
/// Returns `(group_keys, aggregates)`. Both may be empty — callers only push
/// an [`QueryOp::Aggregate`] when at least one is non-empty.
fn lower_aggregate(
    select: &SelectStatement,
    parameters: &[ParameterDescriptor],
) -> Result<(Vec<ProjectField>, Vec<AggregateField>)> {
    let group_keys = match &select.group_by {
        Some(GroupByClause { items, .. }) => {
            let mut keys = Vec::with_capacity(items.len());
            for item in items {
                keys.push(lower_group_key(item)?);
            }
            keys
        }
        None => Vec::new(),
    };

    // Walk SELECT items for aggregate function calls — also capture alias.
    let mut aggregates: Vec<AggregateField> = Vec::new();
    for item in &select.items {
        if let SelectItem::Expression { expr, alias, .. } = item {
            if let Expression::FunctionCall { name, args, .. } = expr {
                let agg = lower_aggregate_function(name, args, alias, parameters)?;
                aggregates.push(agg);
            }
        }
    }

    Ok((group_keys, aggregates))
}

fn lower_group_key(item: &GroupByItem) -> Result<ProjectField> {
    match item {
        GroupByItem::Column(identifier) => {
            let name = identifier.name.to_string();
            let path = vec![name.clone()];
            Ok(ProjectField { name, expr: ProjectExpr::Scalar { path } })
        }
        GroupByItem::Expression(expr) => {
            // Recurse into a plain identifier expression only.
            let Expression::Identifier(identifier) = expr
            else {
                return Err(sql_error("GROUP BY supports plain column references only (complex expressions not yet supported)"));
            };
            let name = identifier.name.to_string();
            let path = vec![name.clone()];
            Ok(ProjectField { name, expr: ProjectExpr::Scalar { path } })
        }
        GroupByItem::Position { index, .. } => {
            Err(sql_error(format!("GROUP BY positional reference (`GROUP BY {index}`) is not yet supported by YYDB MVP")))
        }
    }
}

fn lower_aggregate_function(
    name: &Identifier,
    args: &[Expression],
    alias: &Option<Identifier>,
    parameters: &[ParameterDescriptor],
) -> Result<AggregateField> {
    let fn_name = name.name.to_string().to_ascii_uppercase();
    let output_name = alias.as_ref().map(|a| a.name.to_string()).unwrap_or_else(|| fn_name.to_ascii_lowercase());
    match fn_name.as_str() {
        "COUNT" => {
            if args.is_empty() {
                return Err(sql_error("COUNT() requires an argument (`*` or an expression)"));
            }
            // COUNT(*) — parser may give us a single-star arg or none.
            let first = &args[0];
            match first {
                Expression::Literal(_) | Expression::Identifier(_) | Expression::Binary { .. } => {
                    let expr = lower_project_expr(first, parameters)?;
                    Ok(AggregateField::Count { name: output_name, expr: Some(expr) })
                }
                _ => {
                    // Fallback: treat anything else as COUNT(*).
                    Ok(AggregateField::Count { name: output_name, expr: None })
                }
            }
        }
        "SUM" => {
            if args.len() != 1 {
                return Err(sql_error("SUM() requires exactly one argument"));
            }
            let expr = lower_project_expr(&args[0], parameters)?;
            Ok(AggregateField::Sum { name: output_name, expr })
        }
        "AVG" => {
            if args.len() != 1 {
                return Err(sql_error("AVG() requires exactly one argument"));
            }
            let expr = lower_project_expr(&args[0], parameters)?;
            Ok(AggregateField::Avg { name: output_name, expr })
        }
        other => Err(sql_error(format!("aggregate function `{other}` is not supported by YYDB MVP"))),
    }
}

/// Lower an expression inside SELECT/GROUP BY/aggregate arguments into a
/// [`ProjectExpr`]. Currently supports plain column references only.
fn lower_project_expr(expr: &Expression, _parameters: &[ParameterDescriptor]) -> Result<ProjectExpr> {
    match expr {
        Expression::Identifier(identifier) => {
            let name = identifier.name.to_string();
            Ok(ProjectExpr::Scalar { path: vec![name] })
        }
        _ => Err(sql_error("aggregate arguments support plain column references only")),
    }
}

fn lower_sort_key(item: &OrderByItem) -> Result<SortKey> {
    let Expression::Identifier(identifier) = &item.expr
    else {
        return Err(sql_error("ORDER BY supports plain column references only"));
    };
    Ok(SortKey { field: identifier.name.to_string(), ascending: !matches!(item.direction, oak_sql::ast::OrderDirection::Desc) })
}

fn lower_limit(limit: &LimitClause, parameters: &[ParameterDescriptor], ops: &mut Vec<QueryOp>) -> Result<()> {
    if let Some(offset) = &limit.offset {
        ops.push(QueryOp::Skip { count: lower_bound_value(offset, parameters)? });
    }
    ops.push(QueryOp::Take { count: lower_bound_value(&limit.limit, parameters)? });
    Ok(())
}

/// Returns `None` when the projection is `*` (the scan already yields every
/// stored column). Aggregate function items (`COUNT(...)`, `SUM(...)`) are
/// consumed by the [`QueryOp::Aggregate`] lowering — this function skips them
/// so they do not produce a duplicate Project entry.
fn lower_projection(items: &[SelectItem]) -> Result<Option<Vec<ProjectField>>> {
    if items.len() == 1 {
        if matches!(items[0], SelectItem::Star { .. }) {
            return Ok(None);
        }
    }
    let mut fields = Vec::with_capacity(items.len());
    for item in items {
        let SelectItem::Expression { expr, alias, .. } = item
        else {
            return Err(sql_error("`*` cannot be combined with other select items"));
        };
        // Skip aggregate functions — they are consumed by the Aggregate op.
        if matches!(expr, Expression::FunctionCall { .. }) {
            continue;
        }
        let Expression::Identifier(identifier) = expr
        else {
            return Err(sql_error("select items support plain column references only"));
        };
        let name = alias.as_ref().map_or_else(|| identifier.name.to_string(), |alias| alias.name.to_string());
        fields.push(ProjectField { name, expr: ProjectExpr::Scalar { path: vec![identifier.name.to_string()] } });
    }
    Ok(Some(fields))
}

// ─── predicates ──────────────────────────────────────────────────────────

fn lower_predicate(expr: &Expression, parameters: &[ParameterDescriptor]) -> Result<Pred> {
    match expr {
        Expression::Literal(Literal::Boolean(value, _)) => Ok(if *value { Pred::True } else { Pred::False }),
        Expression::Binary { left, op: BinaryOperator::And, right, .. } => {
            Ok(Pred::And(Box::new(lower_predicate(left, parameters)?), Box::new(lower_predicate(right, parameters)?)))
        }
        Expression::Binary { left, op: BinaryOperator::Or, right, .. } => {
            Ok(Pred::Or(Box::new(lower_predicate(left, parameters)?), Box::new(lower_predicate(right, parameters)?)))
        }
        Expression::Binary { left, op, right, .. } => {
            let cmp = match op {
                BinaryOperator::Equal => CmpOp::Eq,
                BinaryOperator::NotEqual => CmpOp::Ne,
                BinaryOperator::Less => CmpOp::Lt,
                BinaryOperator::LessEqual => CmpOp::Le,
                BinaryOperator::Greater => CmpOp::Gt,
                BinaryOperator::GreaterEqual => CmpOp::Ge,
                BinaryOperator::Like => return Err(sql_error("LIKE is not supported by the YYDB predicate set")),
                _ => return Err(sql_error("arithmetic expressions cannot be used as predicates")),
            };
            let left_path = column_path(left)?;
            // Detect column-vs-column comparisons (JOIN ON). If the right side
            // is a plain identifier that does NOT look like a rewritten param,
            // treat it as a column path instead of a bound value.
            if let Expression::Identifier(right_ident) = right.as_ref() {
                let right_name = right_ident.name.as_ref();
                if !right_name.starts_with("__yydb_param_") {
                    let right_path = vec![right_name.to_string()];
                    return Ok(Pred::FieldCmpField { left_path, op: cmp, right_path });
                }
            }
            let value = lower_bound_value(right, parameters)?;
            Ok(Pred::FieldCmp { path: left_path, op: cmp, value })
        }
        Expression::Unary { op: UnaryOperator::Not, expr, .. } => Ok(Pred::Not(Box::new(lower_predicate(expr, parameters)?))),
        Expression::IsNull { expr, negated, .. } => Ok(Pred::FieldCmp {
            path: column_path(expr)?,
            op: if *negated { CmpOp::Ne } else { CmpOp::Eq },
            value: BoundValue::Literal(Value::Null),
        }),
        Expression::IsTruth { expr, value, negated, .. } => Ok(Pred::FieldCmp {
            path: column_path(expr)?,
            op: if *negated { CmpOp::Ne } else { CmpOp::Eq },
            value: BoundValue::Literal(Value::Bool(*value)),
        }),
        Expression::InList { expr, list, negated, .. } => {
            let path = column_path(expr)?;
            let mut branches = Vec::with_capacity(list.len());
            for item in list {
                branches.push(Pred::FieldCmp { path: path.clone(), op: CmpOp::Eq, value: lower_bound_value(item, parameters)? });
            }
            let mut predicate = branches
                .into_iter()
                .reduce(|left, right| Pred::Or(Box::new(left), Box::new(right)))
                .ok_or_else(|| sql_error("IN requires at least one value"))?;
            if *negated {
                predicate = Pred::Not(Box::new(predicate));
            }
            Ok(predicate)
        }
        Expression::Between { expr, low, high, negated, .. } => {
            let path = column_path(expr)?;
            let mut predicate = Pred::And(
                Box::new(Pred::FieldCmp { path: path.clone(), op: CmpOp::Ge, value: lower_bound_value(low, parameters)? }),
                Box::new(Pred::FieldCmp { path, op: CmpOp::Le, value: lower_bound_value(high, parameters)? }),
            );
            if *negated {
                predicate = Pred::Not(Box::new(predicate));
            }
            Ok(predicate)
        }
        Expression::Identifier(identifier) => {
            Ok(Pred::FieldBool { path: vec![identifier.name.to_string()], value: BoundValue::Literal(Value::Bool(true)) })
        }
        _ => Err(sql_error("unsupported SQL predicate shape")),
    }
}

fn column_path(expr: &Expression) -> Result<Vec<String>> {
    let Expression::Identifier(identifier) = expr
    else {
        return Err(sql_error("predicates support plain column references only"));
    };
    Ok(vec![identifier.name.to_string()])
}

fn lower_bound_value(expr: &Expression, parameters: &[ParameterDescriptor]) -> Result<BoundValue> {
    match expr {
        Expression::Literal(Literal::Null(_)) => Ok(BoundValue::Literal(Value::Null)),
        Expression::Literal(Literal::Boolean(value, _)) => Ok(BoundValue::Literal(Value::Bool(*value))),
        Expression::Literal(Literal::String(text, _)) => Ok(BoundValue::Literal(Value::Text(text.to_string()))),
        Expression::Literal(Literal::Number(text, _)) => Ok(BoundValue::Literal(Value::I64(
            text.parse().map_err(|_| sql_error(format!("only integer literals are supported, got `{text}`")))?,
        ))),
        Expression::Unary { op: UnaryOperator::Minus, expr, .. } => {
            let Expression::Literal(Literal::Number(text, _)) = expr.as_ref()
            else {
                return Err(sql_error("unary minus supports numeric literals only"));
            };
            let number: i64 =
                text.parse().map_err(|_| sql_error(format!("only integer literals are supported, got `{text}`")))?;
            Ok(BoundValue::Literal(Value::I64(-number)))
        }
        Expression::Identifier(identifier) => name_to_slot(&identifier.name, parameters),
        _ => Err(sql_error("unsupported SQL value expression")),
    }
}

/// Lower an SQL expression appearing on the right-hand side of an UPDATE
/// assignment. Unlike [`lower_bound_value`] this permits column references and
/// arithmetic — `UPDATE t SET score = score + 1` must keep the column lookup
/// alive so the executor can read the current row and perform the add.
///
/// Aggregate functions, CASE/CAST, subqueries and other statement-shaped
/// expressions are not supported in write-value context and will be rejected.
fn lower_write_expr(expr: &Expression, parameters: &[ParameterDescriptor]) -> Result<WriteExpr> {
    match expr {
        Expression::Identifier(identifier) => {
            // Identifiers resolve to parameters first (the `__yydb_param_N`
            // convention used by `name_to_slot`), then fall through to a
            // column reference so `score + 1` lowers correctly.
            match name_to_slot(&identifier.name, parameters) {
                Ok(slot) => Ok(WriteExpr::Bound(slot)),
                Err(_) => Ok(WriteExpr::Column { path: vec![identifier.name.to_string()] }),
            }
        }
        Expression::Literal(_) | Expression::Unary { op: UnaryOperator::Minus, .. } => {
            Ok(WriteExpr::Bound(lower_bound_value(expr, parameters)?))
        }
        Expression::Binary { left, op, right, .. } => {
            let bin_op = match op {
                BinaryOperator::Plus => WriteBinOp::Plus,
                BinaryOperator::Minus => WriteBinOp::Minus,
                BinaryOperator::Star => WriteBinOp::Star,
                BinaryOperator::Slash => WriteBinOp::Slash,
                BinaryOperator::Percent => WriteBinOp::Percent,
                _ => return Err(sql_error(format!("arithmetic operator `{op:?}` is not allowed in write expressions"))),
            };
            Ok(WriteExpr::Binary {
                op: bin_op,
                left: Box::new(lower_write_expr(left, parameters)?),
                right: Box::new(lower_write_expr(right, parameters)?),
            })
        }
        _ => Err(sql_error("unsupported expression in UPDATE assignment")),
    }
}

// ─── writes ──────────────────────────────────────────────────────────────

fn lower_insert(insert: &InsertStatement, parameters: &[ParameterDescriptor]) -> Result<WriteOp> {
    if insert.columns.is_empty() {
        return Err(sql_error("INSERT requires an explicit column list"));
    }
    if insert.values.len() != insert.columns.len() {
        return Err(sql_error("INSERT column and value counts differ"));
    }
    let mut fields = Vec::with_capacity(insert.columns.len());
    for (column, value) in insert.columns.iter().zip(&insert.values) {
        fields.push(WriteField { name: column.name.to_string(), value: lower_bound_value(value, parameters)? });
    }
    Ok(WriteOp::Insert { table: table_name(&insert.table_name), fields })
}

fn lower_update(update: &UpdateStatement, parameters: &[ParameterDescriptor]) -> Result<WriteOp> {
    if update.assignments.is_empty() {
        return Err(sql_error("UPDATE requires at least one assignment"));
    }
    let mut assignments = Vec::with_capacity(update.assignments.len());
    for assignment in &update.assignments {
        assignments.push(WriteAssignment {
            field: assignment.column.name.to_string(),
            value: lower_write_expr(&assignment.value, parameters)?,
        });
    }
    let predicate = update.selection.as_ref().map(|expr| lower_predicate(expr, parameters)).transpose()?;
    Ok(WriteOp::Update { table: table_name(&update.table_name), assignments, predicate })
}

fn lower_delete(delete: &DeleteStatement, parameters: &[ParameterDescriptor]) -> Result<WriteOp> {
    let predicate = delete.selection.as_ref().map(|expr| lower_predicate(expr, parameters)).transpose()?;
    Ok(WriteOp::Delete { table: table_name(&delete.table_name), predicate })
}

fn table_name(table: &TableName) -> String {
    table.name.name.to_string()
}
