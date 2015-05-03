//! SQL frontend: lower an Oak SQL AST into the shared physical operators.
//!
//! Parsing belongs to `oak-sql`; this module only *lowers*. It resolves no
//! catalog information and touches no storage, so the same plan can be prepared
//! once and executed against any schema that declares the referenced tables.

use oak_sql::ast::{
    Assignment, BinaryOperator, DeleteStatement, Expression, GroupByClause, GroupByItem, Identifier, InsertSource,
    InsertStatement, JoinClause, JoinType, LimitClause, Literal, OnConflictAction, OrderByItem, SelectItem,
    SelectStatement, SetOperator, SqlRoot, SqlStatement, TableName, UnaryOperator, UpdateStatement,
};
use yydb_types::{Error, Result, Value};

use super::{
    ops::{
        AggregateField, BinaryOp, BoundValue, CmpOp, ConflictAction, ConflictClause, Expr, JoinKind, Pred, ProjectExpr,
        ProjectField, QueryOp, ReturningProjection, SortKey, WriteAssignment, WriteBinOp, WriteExpr, WriteField, WriteOp,
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
                read_set: read_tables_of_write(&op),
                write_ops: vec![op],
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
        SqlStatement::SetOperation(set_operation) => Err(sql_error(format!(
            "{} is not supported by the YYDB query surface: set operations are not lowered yet",
            set_operator_name(set_operation.operator)
        ))),
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

    let mut ops = vec![QueryOp::Scan { table: table.clone() }];

    // JOIN clauses. Self joins stay rejected: joined rows keep the base table's
    // bare column names, so the two sides of one table would be
    // indistinguishable in the ON condition and in the output.
    for join_clause in &select.joins {
        let right = join_clause.table.name.name.to_string();
        if right == table {
            return Err(sql_error(format!(
                "self join on `{right}` is not supported: table aliases cannot tell the two sides apart yet"
            )));
        }
        ops.push(lower_join_clause(join_clause, parameters)?);
    }

    // WHERE clause.
    if let Some(predicate) = &select.expr {
        ops.push(QueryOp::Filter { predicate: lower_predicate(predicate, parameters)? });
    }

    // Aggregate: GROUP BY + aggregate functions from SELECT items.
    // `SELECT DISTINCT` is captured here and, when paired with a COUNT(expr),
    // promotes that COUNT to a distinct-count aggregate. MVP does not yet
    // emit a top-level DISTINCT operator for non-aggregate queries.
    let quantifier_is_distinct = matches!(select.quantifier, Some(oak_sql::ast::SelectQuantifier::Distinct { .. }));
    let (group_keys, aggregates) = lower_aggregate(select, parameters, quantifier_is_distinct)?;
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
        let fields = lower_projection(&select.items, parameters)?;
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
        JoinType::Left => JoinKind::Left,
        JoinType::Right => JoinKind::Right,
        JoinType::Full => JoinKind::Full,
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
    quantifier_is_distinct: bool,
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
                let agg = lower_aggregate_function(name, args, alias, parameters, quantifier_is_distinct)?;
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
    quantifier_is_distinct: bool,
) -> Result<AggregateField> {
    let fn_name = name.name.to_string().to_ascii_uppercase();
    let output_name = alias.as_ref().map(|a| a.name.to_string()).unwrap_or_else(|| fn_name.to_ascii_lowercase());
    match fn_name.as_str() {
        "COUNT" => {
            if args.is_empty() {
                return Err(sql_error("COUNT() requires an argument (`*` or an expression)"));
            }
            let first = &args[0];
            match first {
                Expression::Literal(_) | Expression::Identifier(_) | Expression::Binary { .. } => {
                    let expr = lower_project_expr(first, parameters)?;
                    // `SELECT DISTINCT COUNT(expr)` promotes this COUNT to a
                    // distinct-count aggregate. MVP only handles this pairing
                    // for COUNT; other aggregates ignore the quantifier.
                    if quantifier_is_distinct {
                        Ok(AggregateField::CountDistinct { name: output_name, expr })
                    } else {
                        Ok(AggregateField::Count { name: output_name, expr: Some(expr) })
                    }
                }
                _ => {
                    // Fallback: treat anything else as COUNT(*).
                    if quantifier_is_distinct {
                        return Err(sql_error(
                            "SELECT DISTINCT COUNT(*) is ambiguous (DISTINCT quantifier with COUNT(*)); \
                            provide a column expression or drop DISTINCT",
                        ));
                    }
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
            // Split qualified names (`table.col`) into path components so the
            // executor can resolve both the full dotted key and the trailing
            // bare column name.
            let path: Vec<String> = identifier.name.split('.').map(|s| s.to_string()).collect();
            Ok(ProjectExpr::Scalar { path })
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
fn lower_projection(items: &[SelectItem], parameters: &[ParameterDescriptor]) -> Result<Option<Vec<ProjectField>>> {
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
        let name = output_name(expr, alias);
        // Plain identifier → scalar column path (existing happy path).
        let field_expr = match expr {
            Expression::Identifier(identifier) => {
                ProjectExpr::Scalar { path: vec![identifier.name.to_string()] }
            }
            // Arbitrary lowerable expression (CASE, CAST, nested CASE inside CASE,
            // etc.) → generic Computed wrapper around our Expr IR.
            other => ProjectExpr::Computed(Box::new(lower_expr(other, parameters)?)),
        };
        fields.push(ProjectField { name, expr: field_expr });
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
    // Oak-sql represents qualified names (`table.col`) as a single identifier
    // whose name contains a dot. Split it so the executor can resolve both
    // the full dotted key (right-table columns with prefix) and the trailing
    // bare column (left-table columns without prefix).
    Ok(identifier.name.split('.').map(|s| s.to_string()).collect())
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

/// Lower an arbitrary SQL expression into the backend-neutral [`Expr`] IR.
///
/// Primitive terminals (identifiers → columns, literals and parameter bindings
/// → bound values) and the compound nodes [`Expression::Case`] /
/// [`Expression::Cast`] are supported. Everything else (arithmetic, function
/// calls, subqueries …) is rejected here; callers decide where this IR is
/// allowed — today only SELECT projection uses it.
fn lower_expr(expr: &Expression, parameters: &[ParameterDescriptor]) -> Result<Expr> {
    match expr {
        Expression::Identifier(identifier) => Ok(Expr::Column {
            path: identifier.name.split('.').map(|s| s.to_string()).collect(),
        }),
        Expression::Literal(_) | Expression::Unary { op: UnaryOperator::Minus, .. } => {
            Ok(Expr::Bound(lower_bound_value(expr, parameters)?))
        }
        Expression::Case { branches, else_branch, .. } => {
            let mut lowered_branches = Vec::with_capacity(branches.len());
            for (when, then) in branches {
                lowered_branches.push((
                    Box::new(lower_expr(when, parameters)?),
                    Box::new(lower_expr(then, parameters)?),
                ));
            }
            let lowered_else = else_branch
                .as_ref()
                .map(|els| lower_expr(els, parameters))
                .transpose()?
                .map(Box::new);
            Ok(Expr::Case { branches: lowered_branches, else_branch: lowered_else })
        }
        Expression::Cast { expr: inner, target, .. } => {
            Ok(Expr::Cast {
                expr: Box::new(lower_expr(inner, parameters)?),
                target: target.to_ascii_uppercase(),
            })
        }
        Expression::Binary { left, op, right, .. } => {
            let bin_op = match op {
                BinaryOperator::Equal => BinaryOp::Eq,
                BinaryOperator::NotEqual => BinaryOp::Ne,
                BinaryOperator::Less => BinaryOp::Lt,
                BinaryOperator::LessEqual => BinaryOp::Le,
                BinaryOperator::Greater => BinaryOp::Gt,
                BinaryOperator::GreaterEqual => BinaryOp::Ge,
                BinaryOperator::And => BinaryOp::And,
                BinaryOperator::Or => BinaryOp::Or,
                other => return Err(sql_error(format!("binary operator `{other:?}` is not supported in projection expressions"))),
            };
            Ok(Expr::Binary {
                op: bin_op,
                left: Box::new(lower_expr(left, parameters)?),
                right: Box::new(lower_expr(right, parameters)?),
            })
        }
        Expression::Unary { op: UnaryOperator::Not, expr, .. } => {
            Ok(Expr::Not(Box::new(lower_expr(expr, parameters)?)))
        }
        Expression::IsNull { .. }
        | Expression::IsTruth { .. }
        | Expression::InList { .. }
        | Expression::Between { .. }
        | Expression::InSubquery { .. }
        | Expression::Subquery { .. }
        | Expression::Unary { op: _, expr: _, .. }
        | Expression::FunctionCall { .. }
        | Expression::Wildcard { .. }
        | Expression::Vector { .. }
        | Expression::Error { .. } => Err(sql_error(format!(
            "SQL expression shape {:?} is not supported inside projection expressions",
            match expr {
                Expression::IsNull { .. } => "IS [NOT] NULL",
                Expression::IsTruth { .. } => "IS [NOT] TRUE/FALSE",
                Expression::InList { .. } => "IN (list)",
                Expression::Between { .. } => "BETWEEN",
                Expression::InSubquery { .. } => "IN (subquery)",
                Expression::Subquery { .. } => "subquery",
                Expression::Unary { .. } => "unary",
                Expression::FunctionCall { .. } => "function call",
                Expression::Wildcard { .. } => "*",
                Expression::Vector { .. } => "vector",
                Expression::Error { .. } => "error node",
                _ => "other",
            }
        ))),
    }
}

/// Lower an SQL expression appearing on the right-hand side of an UPDATE
/// assignment (or of `ON CONFLICT … DO UPDATE SET …`). Unlike
/// [`lower_bound_value`] this permits column references and arithmetic —
/// `UPDATE t SET score = score + 1` must keep the column lookup alive so the
/// executor can read the current row and perform the add.
///
/// The accepted surface is literals, parameter slots, plain column references
/// and `+ - * / %` over them. Everything else is rejected rather than
/// approximated: CASE/CAST, function calls, subqueries and comparisons do not
/// lower here, and `excluded.<column>` fails because YYDB does not model the
/// `excluded` pseudo-table (an unmodelled reference would silently read the
/// conflicting row's column instead).
fn lower_write_expr(expr: &Expression, parameters: &[ParameterDescriptor]) -> Result<WriteExpr> {
    match expr {
        Expression::Identifier(identifier) => {
            reject_excluded(&identifier.name)?;
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

/// Reject a reference to the `excluded` pseudo-table in a write expression.
fn reject_excluded(name: &str) -> Result<()> {
    let root = name.split('.').next().unwrap_or(name);
    if root.eq_ignore_ascii_case("excluded") {
        return Err(sql_error(format!(
            "`{name}` references `excluded`, which YYDB does not model: write the inserted value as a literal or parameter"
        )));
    }
    Ok(())
}

// ─── writes ──────────────────────────────────────────────────────────────

fn lower_insert(insert: &InsertStatement, parameters: &[ParameterDescriptor]) -> Result<WriteOp> {
    let conflict = lower_conflict(insert, parameters)?;
    let returning = lower_returning(insert, parameters)?;
    match &insert.source {
        InsertSource::Select(select) => lower_insert_select(insert, select, conflict, returning, parameters),
        InsertSource::Values => lower_insert_values(insert, conflict, returning, parameters),
    }
}

/// Lower an `RETURNING` clause into the shared projection.
///
/// The items reuse the SELECT projection lowering, so `*`, aliases and the
/// scalar expression shapes it accepts stay identical for reads and writes.
/// Aggregate functions are rejected instead of being skipped, because a
/// `RETURNING COUNT(*)` would otherwise hand back an empty row.
fn lower_returning(
    insert: &InsertStatement,
    parameters: &[ParameterDescriptor],
) -> Result<Option<ReturningProjection>> {
    let Some(clause) = &insert.returning
    else {
        return Ok(None);
    };
    if clause.items.is_empty() {
        return Err(sql_error("RETURNING requires at least one item"));
    }
    if clause.items.iter().any(|item| matches!(item, SelectItem::Expression { expr: Expression::FunctionCall { .. }, .. })) {
        return Err(sql_error(
            "RETURNING supports columns and scalar expressions: aggregate functions are not supported",
        ));
    }
    Ok(match lower_projection(&clause.items, parameters)? {
        Some(fields) => Some(ReturningProjection::Fields(fields)),
        None => Some(ReturningProjection::Star),
    })
}

/// Lower an `ON CONFLICT` clause into the shared conflict operator.
///
/// The target columns stay as written; resolving them against the primary key
/// or a uniqueness rule is execution's job, because lowering has no catalog.
fn lower_conflict(insert: &InsertStatement, parameters: &[ParameterDescriptor]) -> Result<Option<ConflictClause>> {
    let Some(clause) = &insert.on_conflict
    else {
        return Ok(None);
    };
    let mut columns: Vec<String> = Vec::with_capacity(clause.columns.len());
    for identifier in &clause.columns {
        let column = conflict_target_column(&identifier.name, &table_name(&insert.table_name))?;
        if columns.contains(&column) {
            return Err(sql_error(format!("ON CONFLICT target column `{column}` appears more than once")));
        }
        columns.push(column);
    }
    let action = match &clause.action {
        OnConflictAction::DoNothing => ConflictAction::DoNothing,
        OnConflictAction::DoUpdate { assignments, selection } => ConflictAction::DoUpdate {
            assignments: lower_conflict_assignments(assignments, parameters)?,
            predicate: selection
                .as_ref()
                .map(|expr| lower_conflict_predicate(expr, parameters))
                .transpose()?,
        },
    };
    Ok(Some(ConflictClause { columns, action }))
}

/// Lower the `SET` list of `ON CONFLICT … DO UPDATE`.
///
/// The assignments reuse the update write-expression surface, so an expression
/// the executor cannot evaluate is rejected instead of being approximated.
fn lower_conflict_assignments(
    assignments: &[Assignment],
    parameters: &[ParameterDescriptor],
) -> Result<Vec<WriteAssignment>> {
    if assignments.is_empty() {
        return Err(sql_error("ON CONFLICT DO UPDATE requires at least one assignment"));
    }
    let mut lowered = Vec::with_capacity(assignments.len());
    for assignment in assignments {
        lowered.push(WriteAssignment {
            field: assignment.column.name.to_string(),
            value: lower_write_expr(&assignment.value, parameters)?,
        });
    }
    Ok(lowered)
}

/// Lower the optional `WHERE` of `ON CONFLICT … DO UPDATE`.
///
/// The predicate is evaluated against the conflicting row, so a reference to
/// the `excluded` pseudo-table is rejected: YYDB does not model it, and the
/// qualified identifier would otherwise silently fall through to the row's own
/// column of the same name.
fn lower_conflict_predicate(expr: &Expression, parameters: &[ParameterDescriptor]) -> Result<Pred> {
    let predicate = lower_predicate(expr, parameters)?;
    reject_excluded_paths(&predicate)?;
    Ok(predicate)
}

fn reject_excluded_paths(predicate: &Pred) -> Result<()> {
    fn check(path: &[String]) -> Result<()> {
        match path.first() {
            Some(first) if first.eq_ignore_ascii_case("excluded") => Err(sql_error(
                "`excluded` is not modeled by YYDB: name the conflicting row's own columns instead of the pseudo-table",
            )),
            _ => Ok(()),
        }
    }
    match predicate {
        Pred::True | Pred::False => Ok(()),
        Pred::FieldBool { path, .. } => check(path),
        Pred::FieldCmp { path, .. } => check(path),
        Pred::FieldCmpField { left_path, right_path, .. } => {
            check(left_path)?;
            check(right_path)
        }
        Pred::And(left, right) | Pred::Or(left, right) => {
            reject_excluded_paths(left)?;
            reject_excluded_paths(right)
        }
        Pred::Not(inner) => reject_excluded_paths(inner),
    }
}

/// Normalise one written conflict target: the bare column, or a table-qualified
/// reference that must name the statement's own target table.
fn conflict_target_column(written: &str, table: &str) -> Result<String> {
    let parts: Vec<&str> = written.split('.').collect();
    match parts.as_slice() {
        [column] => Ok((*column).to_string()),
        [prefix, column] if *prefix == table => Ok((*column).to_string()),
        _ => Err(sql_error(format!(
            "ON CONFLICT target `{written}` must be a column of the insert target `{table}`"
        ))),
    }
}

fn lower_insert_values(
    insert: &InsertStatement,
    conflict: Option<ConflictClause>,
    returning: Option<ReturningProjection>,
    parameters: &[ParameterDescriptor],
) -> Result<WriteOp> {
    if insert.columns.is_empty() {
        return Err(sql_error("INSERT requires an explicit column list"));
    }
    if insert.rows.is_empty() {
        return Err(sql_error("INSERT requires at least one VALUES row"));
    }
    let mut rows = Vec::with_capacity(insert.rows.len());
    for (index, values) in insert.rows.iter().enumerate() {
        if values.len() != insert.columns.len() {
            return Err(sql_error(format!(
                "INSERT column and value counts differ in VALUES row {}: {} columns, {} values",
                index + 1,
                insert.columns.len(),
                values.len()
            )));
        }
        let mut fields = Vec::with_capacity(insert.columns.len());
        for (column, value) in insert.columns.iter().zip(values) {
            fields.push(WriteField { name: column.name.to_string(), value: lower_bound_value(value, parameters)? });
        }
        rows.push(fields);
    }
    Ok(WriteOp::Insert { table: table_name(&insert.table_name), rows, conflict, returning })
}

/// `INSERT INTO … SELECT …`: lower the source into the shared read pipeline and
/// hand execution the target columns plus the source output order.
///
/// Rows are mapped to target columns by position, so the source must be a plain
/// row projection whose output names are unique. Aggregates, `GROUP BY` and
/// `DISTINCT` change how the pipeline shapes its output row and stay rejected
/// instead of being approximated.
fn lower_insert_select(
    insert: &InsertStatement,
    select: &SelectStatement,
    conflict: Option<ConflictClause>,
    returning: Option<ReturningProjection>,
    parameters: &[ParameterDescriptor],
) -> Result<WriteOp> {
    if select.group_by.is_some() {
        return Err(sql_error("INSERT ... SELECT source must be a plain row projection: GROUP BY is not supported yet"));
    }
    if matches!(select.quantifier, Some(oak_sql::ast::SelectQuantifier::Distinct { .. })) {
        return Err(sql_error("INSERT ... SELECT source must be a plain row projection: DISTINCT is not supported yet"));
    }
    if select.items.iter().any(|item| matches!(item, SelectItem::Expression { expr: Expression::FunctionCall { .. }, .. })) {
        return Err(sql_error("INSERT ... SELECT source must be a plain row projection: aggregate functions are not supported yet"));
    }

    let outputs = select_output_names(&select.items)?;
    let source = lower_select(select, parameters)?;
    let columns: Vec<String> = insert.columns.iter().map(|column| column.name.to_string()).collect();
    if let Some(outputs) = &outputs
        && !columns.is_empty()
        && columns.len() != outputs.len()
    {
        return Err(insert_select_column_mismatch(columns.len(), outputs.len()));
    }
    Ok(WriteOp::InsertFromQuery { table: table_name(&insert.table_name), columns, source, outputs, conflict, returning })
}

/// Projection name of one SELECT item: the alias when written, otherwise the
/// column name, the shape keyword, or a generic `expr` placeholder.
fn output_name(expr: &Expression, alias: &Option<Identifier>) -> String {
    alias.as_ref().map_or_else(
        || match expr {
            Expression::Identifier(ident) => ident.name.to_string(),
            Expression::Case { .. } => "case".to_string(),
            Expression::Cast { .. } => "cast".to_string(),
            _ => "expr".to_string(),
        },
        |alias| alias.name.to_string(),
    )
}

/// Output column names of a SELECT list, in projection order.
///
/// Returns `None` for `SELECT *`, whose columns are only known once the scanned
/// table is resolved against the catalog.
fn select_output_names(items: &[SelectItem]) -> Result<Option<Vec<String>>> {
    if items.len() == 1
        && matches!(items[0], SelectItem::Star { .. })
    {
        return Ok(None);
    }
    let mut names = Vec::with_capacity(items.len());
    for item in items {
        let SelectItem::Expression { expr, alias, .. } = item
        else {
            return Err(sql_error("`*` cannot be combined with other select items"));
        };
        let name = output_name(expr, alias);
        if names.contains(&name) {
            return Err(sql_error(format!(
                "INSERT ... SELECT output column `{name}` appears more than once: give the source columns distinct aliases so they map to target columns by position"
            )));
        }
        names.push(name);
    }
    Ok(Some(names))
}

fn insert_select_column_mismatch(target: usize, source: usize) -> Error {
    sql_error(format!("INSERT ... SELECT column count mismatch: {target} target columns, {source} source columns"))
}

/// Tables a write operator reads, used to populate [`SqlPlan::read_set`].
fn read_tables_of_write(op: &WriteOp) -> Vec<String> {
    let WriteOp::InsertFromQuery { source, .. } = op
    else {
        return Vec::new();
    };
    let mut tables: Vec<String> = Vec::new();
    for candidate in source {
        let table = match candidate {
            QueryOp::Scan { table } => table,
            QueryOp::Join { right, .. } => right,
            _ => continue,
        };
        if !tables.contains(table) {
            tables.push(table.clone());
        }
    }
    tables
}

fn set_operator_name(operator: SetOperator) -> &'static str {
    match operator {
        SetOperator::Union => "UNION",
        SetOperator::UnionAll => "UNION ALL",
        SetOperator::Intersect => "INTERSECT",
        SetOperator::Except => "EXCEPT",
    }
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
