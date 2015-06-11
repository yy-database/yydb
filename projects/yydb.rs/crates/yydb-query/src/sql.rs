//! SQL frontend: lower an Oak SQL AST into the shared physical operators.
//!
//! Parsing belongs to `oak-sql`; this module only *lowers*. It resolves no
//! catalog information and touches no storage, so the same plan can be prepared
//! once and executed against any schema that declares the referenced tables.

use std::{cell::Cell, collections::BTreeMap, rc::Rc};

use oak_sql::ast::{
    Assignment, BinaryOperator, DeleteStatement, Expression, GroupByClause, GroupByItem, Identifier, InsertSource,
    InsertStatement, JoinClause, JoinType, LimitClause, Literal, OnConflictAction, OrderByItem, SelectItem, SelectStatement,
    SetOperationStatement, SetOperator, SqlRoot, SqlStatement, TableName, TableSource, TransactionAction, UnaryOperator,
    UpdateStatement, WindowFrame as AstWindowFrame, WindowFrameBound as AstWindowFrameBound, WindowFrameUnit as AstWindowFrameUnit,
    WindowOrderItem, WindowSpec as AstWindowSpec, WithClause,
};
use yydb_types::{Error, Result, Value};

use super::{
    ops::{
        AggregateField, BinaryOp, BoundValue, CmpOp, ConflictAction, ConflictClause, CountExpr, Expr, JoinKind,
        OUTER_ROW_MARKER, Pred, ProjectExpr, ProjectField, QueryOp, ReturningProjection, ScalarFunction,
        SessionOp, SetOpKind, SortKey, WindowField, WindowFrame, WindowFrameBound, WindowFunction, WindowOrderKey,
        WindowSpec, WriteAssignment, WriteBinOp, WriteExpr, WriteField, WriteOp,
    },
    params::{PARAM_PREFIX, ParameterDescriptor, name_to_slot},
};

fn sql_error(message: impl Into<String>) -> Error {
    Error::Schema { message: message.into() }
}

/// One lowered SQL program: either a read pipeline, a write op list, or a
/// session/transaction boundary op.
#[derive(Debug, Clone, PartialEq)]
pub struct SqlPlan {
    /// Read pipeline ending in [`QueryOp::Collect`]; empty for writes and session ops.
    pub read_ops: Vec<QueryOp>,
    /// Write operators; empty for reads and session ops.
    pub write_ops: Vec<WriteOp>,
    /// Session / transaction boundary operator list (empty for read and write
    /// statements). Always at most one op — SQL source must contain exactly
    /// one statement, and session statements lower to a single op.
    pub session_ops: Vec<SessionOp>,
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
            let (ops, _) = lower_select(select, parameters, &CteScope::new())?;
            let mut read_set = Vec::new();
            collect_read_tables(&ops, &mut read_set);
            Ok(SqlPlan { read_ops: ops, write_ops: Vec::new(), session_ops: Vec::new(), read_set, write_set: Vec::new() })
        }
        SqlStatement::Insert(insert) => {
            let op = lower_insert(insert, parameters)?;
            Ok(SqlPlan {
                read_ops: Vec::new(),
                read_set: read_tables_of_write(&op),
                write_ops: vec![op],
                write_set: vec![insert.table_name.name.name.to_string()],
                session_ops: Vec::new(),
            })
        }
        SqlStatement::Update(update) => {
            let op = lower_update(update, parameters)?;
            Ok(SqlPlan {
                read_ops: Vec::new(),
                write_ops: vec![op],
                read_set: Vec::new(),
                write_set: vec![update.table_name.name.name.to_string()],
                session_ops: Vec::new(),
            })
        }
        SqlStatement::Delete(delete) => {
            let op = lower_delete(delete, parameters)?;
            Ok(SqlPlan {
                read_ops: Vec::new(),
                write_ops: vec![op],
                read_set: Vec::new(),
                write_set: vec![delete.table_name.name.name.to_string()],
                session_ops: Vec::new(),
            })
        }
        SqlStatement::SetOperation(set_operation) => {
            let ops = lower_set_operation(set_operation, parameters, &CteScope::new())?;
            let mut read_set = Vec::new();
            collect_read_tables(&ops, &mut read_set);
            Ok(SqlPlan { read_ops: ops, write_ops: Vec::new(), session_ops: Vec::new(), read_set, write_set: Vec::new() })
        }
        SqlStatement::Transaction(transaction) => {
            let op = match &transaction.action {
                TransactionAction::Begin => SessionOp::Begin,
                TransactionAction::Commit => SessionOp::Commit,
                TransactionAction::Rollback => SessionOp::Rollback,
                TransactionAction::Savepoint(name) => SessionOp::Savepoint(name.to_string()),
                TransactionAction::RollbackToSavepoint(name) => SessionOp::RollbackToSavepoint(name.to_string()),
                TransactionAction::ReleaseSavepoint(name) => SessionOp::ReleaseSavepoint(name.to_string()),
            };
            Ok(SqlPlan {
                read_ops: Vec::new(),
                write_ops: Vec::new(),
                session_ops: vec![op],
                read_set: Vec::new(),
                write_set: Vec::new(),
            })
        }
        SqlStatement::Error { message, .. } => Err(sql_error(format!("SQL statement error: {message}"))),
        SqlStatement::Create(_) | SqlStatement::Drop(_) | SqlStatement::Alter(_) => {
            Err(sql_error("DDL is owned by the schema document, not by SQL statements"))
        }
        SqlStatement::SetNames(_) | SqlStatement::SetVariable(_) => {
            Err(sql_error("session variable statements are not part of the YYDB SQL query surface"))
        }
        SqlStatement::Show(_) | SqlStatement::Pragma(_) => {
            Err(sql_error("session introspection is not part of the YYDB SQL query surface"))
        }
        SqlStatement::Unknown { .. } => Err(sql_error("unknown SQL statement")),
    }
}

// ─── read pipeline ───────────────────────────────────────────────────────

/// One enclosing statement a nested subquery may read columns from.
#[derive(Debug)]
struct OuterFrame {
    /// Row-source references that statement exposes to its own expressions.
    references: Vec<String>,
    /// Whether a bare column reference is ambiguous in that statement.
    require_qualification: bool,
    /// Whether that statement collapses its rows through `GROUP BY` or an
    /// aggregate function, which leaves no single row to hand to a subquery.
    aggregate: bool,
    /// Set once one of that statement's own expressions reads a column of the
    /// statement enclosing it, which makes it a correlated statement.
    reads_outer: Cell<bool>,
}

/// Lowering scope of one statement.
///
/// `pipelines` carries the `WITH` bodies already lowered. `frames` is the chain
/// of enclosing statements, innermost last: a subquery that names a reference of
/// an outer frame is correlated and is executed once per row of that statement,
/// so every carrier it reaches is lowered with `correlated: true`.
#[derive(Debug, Clone, Default)]
struct CteScope {
    pipelines: BTreeMap<String, Vec<QueryOp>>,
    frames: Vec<Rc<OuterFrame>>,
}

impl CteScope {
    fn new() -> Self {
        Self::default()
    }

    /// Whether `name` reads a column of an enclosing statement.
    ///
    /// Only asked after [`check_select_references`] accepted the reference, so
    /// the answer agrees with the correlation it recorded.
    fn reads_outer_row(&self, name: &str) -> bool {
        let Some(local) = self.frames.last()
        else {
            return false;
        };
        let Some((qualifier, _)) = name.split_once('.')
        else {
            return false;
        };
        if local.references.iter().any(|reference| reference == qualifier) {
            return false;
        }
        self.frames[..self.frames.len() - 1].iter().any(|frame| frame.references.iter().any(|reference| reference == qualifier))
    }
}

/// Whether a statement collapses its rows, so a subquery cannot be handed one
/// row of it. Mirrors the aggregates [`lower_aggregate`] accepts.
fn select_has_aggregate(select: &SelectStatement) -> bool {
    select.group_by.is_some()
        || select.items.iter().any(|item| {
            matches!(
                item,
                SelectItem::Expression { expr: Expression::FunctionCall { name, window: None, .. }, .. }
                    if is_aggregate_function(name)
            )
        })
}

fn lower_select(select: &SelectStatement, parameters: &[ParameterDescriptor], ctes: &CteScope) -> Result<(Vec<QueryOp>, bool)> {
    // A `WITH` clause extends the visible scope in written order: each CTE may
    // reference the ones written before it, and the statement body sees all of
    // them.
    let mut scope = ctes.clone();
    extend_cte_scope(&mut scope, select.with.as_ref(), parameters)?;

    let from = select.from.as_ref().ok_or_else(|| sql_error("SELECT requires a FROM clause"))?;
    let base = row_source(from, parameters, &scope)?;

    // A derived table or a `WITH` name contributes its sub-pipeline verbatim,
    // followed by a `Qualify` when the statement references it through an alias.
    // A plain table contributes a `Scan`, qualified under the same rule.
    let mut ops = match &base {
        RowSource::Table { table, alias } => {
            let mut ops = vec![QueryOp::Scan { table: table.clone() }];
            if alias.is_some() {
                ops.push(QueryOp::Qualify { prefix: base.reference() });
            }
            ops
        }
        RowSource::Derived { alias, ops: inner } => {
            let mut ops = nested_ops(inner);
            ops.push(QueryOp::Qualify { prefix: alias.clone() });
            ops
        }
        RowSource::Cte { alias, ops: inner, .. } => {
            let mut ops = nested_ops(inner);
            if alias.is_some() {
                ops.push(QueryOp::Qualify { prefix: base.reference() });
            }
            ops
        }
    };

    // JOIN clauses. Every row source is referenced by its alias when it has one,
    // so a table may join itself as long as both occurrences are aliased
    // differently. Two sources sharing one reference stay rejected: nothing in
    // the statement could tell their columns apart.
    let mut references = vec![base.reference()];
    let mut aliased = base.is_aliased();
    for join_clause in &select.joins {
        let right = row_source(&join_clause.table, parameters, &scope)?;
        let reference = right.reference();
        if references.contains(&reference) {
            return Err(sql_error(format!(
                "`{reference}` is referenced more than once: give each occurrence a distinct alias (`FROM a AS x JOIN b AS y …`)"
            )));
        }
        aliased |= right.is_aliased();
        references.push(reference);
        ops.push(lower_join_clause(join_clause, right, parameters, &scope)?);
    }

    // This statement's own frame decides what its nested subqueries may read, and
    // records whether it reads a column of the statement enclosing it — which is
    // what becomes `correlated` on the carrier that reaches it.
    let frame = Rc::new(OuterFrame {
        references: references.clone(),
        require_qualification: aliased,
        aggregate: select_has_aggregate(select),
        reads_outer: Cell::new(false),
    });
    scope.frames.push(Rc::clone(&frame));

    // Aliases are the only disambiguator, so an aliased statement must reach its
    // columns through them: a bare `id` would silently bind to whichever
    // occurrence happens to carry it. Statements without aliases still reject
    // qualifiers that name no row source of their own scope.
    check_select_references(select, &scope)?;

    // WHERE clause.
    if let Some(predicate) = &select.expr {
        ops.push(QueryOp::Filter { predicate: lower_predicate(predicate, parameters, &scope)? });
    }

    // Aggregate: GROUP BY + aggregate functions from SELECT items.
    // `SELECT DISTINCT` is captured here and, when paired with a COUNT(expr),
    // promotes that COUNT to a distinct-count aggregate. MVP does not yet
    // emit a top-level DISTINCT operator for non-aggregate queries.
    let quantifier_is_distinct = matches!(select.quantifier, Some(oak_sql::ast::SelectQuantifier::Distinct { .. }));
    let (group_keys, aggregates) = lower_aggregate(select, parameters, quantifier_is_distinct, &scope)?;
    let has_aggregate = !aggregates.is_empty() || !group_keys.is_empty();
    if has_aggregate {
        ops.push(QueryOp::Aggregate { group_keys, aggregates });
    }

    // HAVING — MVP rejects until Aggregate + Filter composition lands.
    if select.having.is_some() {
        return Err(sql_error("HAVING clause is not yet supported by the YYDB query executor"));
    }

    // Window functions. Every `OVER (...)` item is computed over the filtered
    // rows before ORDER BY, which is why a window cannot be combined with
    // GROUP BY: the aggregate op replaces the rows the window would read.
    let windows = lower_windows(select, parameters, &scope)?;
    if !windows.is_empty() {
        if has_aggregate {
            return Err(sql_error(
                "window functions cannot be combined with `GROUP BY` or aggregate functions in the same SELECT",
            ));
        }
        ops.push(QueryOp::Window { fields: windows });
    }

    // ORDER BY.
    if let Some(order_by) = &select.order_by {
        let mut keys = Vec::with_capacity(order_by.items.len());
        for item in &order_by.items {
            keys.push(lower_sort_key(item, parameters, &scope)?);
        }
        ops.push(QueryOp::Sort { keys });
    }

    // LIMIT.
    if let Some(limit) = &select.limit {
        lower_limit(limit, parameters, &scope, &mut ops)?;
    }

    // SELECT projection (without aggregate function items — those are
    // consumed by the Aggregate op above). When an Aggregate op was pushed
    // the row already carries every output column with its final name (GROUP
    // BY columns + aggregate aliases), so we skip Project entirely —
    // pushing Project with only the non-aggregate items would drop the
    // aggregate columns.
    if !has_aggregate {
        let fields = lower_projection(&select.items, parameters, &scope)?;
        if let Some(fields) = fields
            && !fields.is_empty()
        {
            ops.push(QueryOp::Project { fields });
        }
    }
    ops.push(QueryOp::Collect);
    Ok((ops, frame.reads_outer.get()))
}

/// Extend one statement's `WITH` scope with the clause's definitions.
///
/// The definitions are lowered in written order, so each one may reference the
/// CTEs above it. Renaming a CTE's output columns is not modelled yet.
fn extend_cte_scope(scope: &mut CteScope, with_clause: Option<&WithClause>, parameters: &[ParameterDescriptor]) -> Result<()> {
    let Some(with_clause) = with_clause
    else {
        return Ok(());
    };
    for cte in &with_clause.ctes {
        let name = cte.name.name.to_string();
        if scope.pipelines.contains_key(&name) {
            return Err(sql_error(format!("`{name}` is defined more than once in the same `WITH` scope")));
        }
        if !cte.columns.is_empty() {
            return Err(sql_error(format!(
                "renaming the output columns of CTE `{name}` (`WITH {name}(a, b) AS …`) is not supported yet"
            )));
        }
        // A `WITH` body is a statement of its own, so it cannot read the row of
        // the statement that writes the clause.
        let mut body = scope.clone();
        body.frames.clear();
        let (ops, _) = lower_select(&cte.query, parameters, &body)?;
        scope.pipelines.insert(name, ops);
    }
    Ok(())
}

// ─── set operations ──────────────────────────────────────────────────────

/// Lower one `UNION` / `INTERSECT` / `EXCEPT` statement.
///
/// The AST keeps the two operands as plain `SELECT` statements, which
/// guarantees a single binary operator per statement. `ORDER BY` and `LIMIT`
/// sit on the set operation node because they apply to the combined result, so
/// they are pushed after the [`QueryOp::SetOperation`] that merges both
/// branches.
///
/// Oak-sql attaches a written `WITH` clause to the left operand only. The clause
/// is lowered once here and handed to both branches, so a CTE stays visible to
/// the right operand of the same statement.
fn lower_set_operation(
    statement: &SetOperationStatement,
    parameters: &[ParameterDescriptor],
    ctes: &CteScope,
) -> Result<Vec<QueryOp>> {
    let mut scope = ctes.clone();

    // Lower the first operand and extend CTE scope from it (Oak attaches WITH
    // only to the leftmost operand; we surface those CTEs to every operand).
    let mut ops: Vec<QueryOp> = {
        let first = &statement.operands[0];
        let mut left = (**first).clone();
        extend_cte_scope(&mut scope, left.with.take().as_ref(), parameters)?;
        lower_select(&left, parameters, &scope)?.0
    };
    // Output columns are anchored to the first operand and never change as we
    // fold subsequent operands — every set operation keeps the left side's
    // column names.
    let columns = pipeline_output_columns(&ops);

    // Fold remaining operands into the pipeline as nested SetOperation nodes.
    // `operands.len() == operators.len() + 1` by invariant.
    for (i, operator) in statement.operators.iter().enumerate() {
        let right = lower_select(&statement.operands[i + 1], parameters, &scope)?.0;
        let right_columns = pipeline_output_columns(&right);

        if let (Some(c), Some(rc)) = (&columns, &right_columns)
            && c.len() != rc.len()
        {
            return Err(set_operation_column_mismatch(*operator, c.len(), rc.len()));
        }

        ops = vec![QueryOp::SetOperation {
            kind: set_operation_kind(*operator),
            left: ops,
            right,
            columns: columns.clone(),
            right_columns,
        }];
    }

    // The combined rows carry the left operand's column names, so an `ORDER BY`
    // may only name one of them. Sorting on an unknown name would silently read
    // `NULL` instead of failing. Sorting by a scalar expression is fine — the
    // executor evaluates it per row.
    if let (Some(columns), Some(order_by)) = (&columns, &statement.order_by) {
        for item in &order_by.items {
            let key = lower_sort_key(item, parameters, &scope)?;
            if key.expr.is_none() && !columns.contains(&key.field) {
                return Err(sql_error(format!(
                    "ORDER BY column `{}` is not an output column of this {} (output columns: {})",
                    key.field,
                    set_operator_name(*statement.operators.first().unwrap_or(&SetOperator::Union)),
                    columns.join(", ")
                )));
            }
        }
    }

    let mut final_ops = ops;
    if let Some(order_by) = &statement.order_by {
        let mut keys = Vec::with_capacity(order_by.items.len());
        for item in &order_by.items {
            keys.push(lower_sort_key(item, parameters, &scope)?);
        }
        final_ops.push(QueryOp::Sort { keys });
    }
    if let Some(limit) = &statement.limit {
        lower_limit(limit, parameters, &scope, &mut final_ops)?;
    }
    final_ops.push(QueryOp::Collect);
    Ok(final_ops)
}

fn set_operation_column_mismatch(operator: SetOperator, left: usize, right: usize) -> Error {
    sql_error(format!(
        "{} operands must produce the same number of columns: {left} on the left, {right} on the right",
        set_operator_name(operator)
    ))
}

fn set_operation_kind(operator: SetOperator) -> SetOpKind {
    match operator {
        SetOperator::Union => SetOpKind::Union,
        SetOperator::UnionAll => SetOpKind::UnionAll,
        SetOperator::Intersect => SetOpKind::Intersect,
        SetOperator::Except => SetOpKind::Except,
    }
}

/// Output columns of one lowered read pipeline, in projection order.
///
/// A `Project` names them; an `Aggregate` names them as its group keys followed
/// by its aggregate outputs, because projection is skipped in that case. `None`
/// means the pipeline projects `*` and keeps whatever the scan loaded.
fn pipeline_output_columns(ops: &[QueryOp]) -> Option<Vec<String>> {
    for op in ops.iter().rev() {
        match op {
            QueryOp::Project { fields } => return Some(fields.iter().map(|field| field.name.clone()).collect()),
            QueryOp::Aggregate { group_keys, aggregates } => {
                let mut names: Vec<String> = group_keys.iter().map(|key| key.name.clone()).collect();
                names.extend(aggregates.iter().map(|agg| agg.name().to_string()));
                return Some(names);
            }
            QueryOp::Filter { .. } | QueryOp::Sort { .. } | QueryOp::Skip { .. } | QueryOp::Take { .. } | QueryOp::Collect => {
                continue;
            }
            _ => return None,
        }
    }
    None
}

// ─── join lowering ───────────────────────────────────────────────────────

fn lower_join_clause(
    join: &JoinClause,
    right: RowSource,
    parameters: &[ParameterDescriptor],
    ctes: &CteScope,
) -> Result<QueryOp> {
    let kind = match join.join_type {
        JoinType::Inner => JoinKind::Inner,
        JoinType::Left => JoinKind::Left,
        JoinType::Right => JoinKind::Right,
        JoinType::Full => JoinKind::Full,
        JoinType::Cross => JoinKind::Cross,
    };
    let on = join.on.as_ref().map(|expr| lower_predicate(expr, parameters, ctes)).transpose()?.unwrap_or(Pred::True);
    Ok(match right {
        RowSource::Table { table, alias } => QueryOp::Join { kind, right: table, right_alias: alias, on },
        RowSource::Derived { alias, ops } => QueryOp::JoinSubquery { kind, right: ops, right_alias: alias, on },
        RowSource::Cte { name, alias, ops } => {
            QueryOp::JoinSubquery { kind, right: ops, right_alias: alias.unwrap_or(name), on }
        }
    })
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
    ctes: &CteScope,
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
    // A windowed call (`SUM(x) OVER (...)`) is computed by the `Window` op
    // instead, so it must not be lowered as a plain aggregate here.
    let mut aggregates: Vec<AggregateField> = Vec::new();
    for item in &select.items {
        if let SelectItem::Expression { expr, alias, .. } = item
            && let Expression::FunctionCall { name, args, window: None, .. } = expr
            && is_aggregate_function(name)
        {
            let agg = lower_aggregate_function(name, args, alias, parameters, quantifier_is_distinct, ctes)?;
            aggregates.push(agg);
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
            if let Some(name) = first_window_function(expr) {
                return Err(sql_error(format!("window function `{name}` cannot appear in `GROUP BY`")));
            }
            // Recurse into a plain identifier expression only.
            let Expression::Identifier(identifier) = expr
            else {
                return Err(sql_error(
                    "GROUP BY supports plain column references only (complex expressions not yet supported)",
                ));
            };
            let name = identifier.name.to_string();
            let path = vec![name.clone()];
            Ok(ProjectField { name, expr: ProjectExpr::Scalar { path } })
        }
        GroupByItem::Position { index, .. } => {
            Err(sql_error(format!("GROUP BY positional reference (`GROUP BY {index}`) is not yet supported by YYDB MVP")))
        }
        GroupByItem::GroupingSets(_) | GroupByItem::Rollup(_) | GroupByItem::Cube(_) => {
            Err(sql_error("GROUPING SETS / ROLLUP / CUBE are not yet supported by YYDB MVP"))
        }
    }
}

fn lower_aggregate_function(
    name: &Identifier,
    args: &[Expression],
    alias: &Option<Identifier>,
    parameters: &[ParameterDescriptor],
    quantifier_is_distinct: bool,
    ctes: &CteScope,
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
                Expression::Wildcard { .. } => {
                    if quantifier_is_distinct {
                        return Err(sql_error(
                            "SELECT DISTINCT COUNT(*) is ambiguous (DISTINCT quantifier with COUNT(*)); \
                            provide a column expression or drop DISTINCT",
                        ));
                    }
                    Ok(AggregateField::Count { name: output_name, expr: None })
                }
                other => {
                    let expr = lower_project_expr(other, parameters, ctes)?;
                    // `SELECT DISTINCT COUNT(expr)` promotes this COUNT to a
                    // distinct-count aggregate. MVP only handles this pairing
                    // for COUNT; other aggregates ignore the quantifier.
                    if quantifier_is_distinct {
                        Ok(AggregateField::CountDistinct { name: output_name, expr })
                    }
                    else {
                        Ok(AggregateField::Count { name: output_name, expr: Some(expr) })
                    }
                }
            }
        }
        "SUM" => {
            if args.len() != 1 {
                return Err(sql_error("SUM() requires exactly one argument"));
            }
            let expr = lower_project_expr(&args[0], parameters, ctes)?;
            Ok(AggregateField::Sum { name: output_name, expr })
        }
        "AVG" => {
            if args.len() != 1 {
                return Err(sql_error("AVG() requires exactly one argument"));
            }
            let expr = lower_project_expr(&args[0], parameters, ctes)?;
            Ok(AggregateField::Avg { name: output_name, expr })
        }
        "MIN" => {
            if args.len() != 1 {
                return Err(sql_error("MIN() requires exactly one argument"));
            }
            let expr = lower_project_expr(&args[0], parameters, ctes)?;
            Ok(AggregateField::Min { name: output_name, expr })
        }
        "MAX" => {
            if args.len() != 1 {
                return Err(sql_error("MAX() requires exactly one argument"));
            }
            let expr = lower_project_expr(&args[0], parameters, ctes)?;
            Ok(AggregateField::Max { name: output_name, expr })
        }
        other => Err(sql_error(format!("aggregate function `{other}` is not supported by YYDB MVP"))),
    }
}

// ─── window lowering ─────────────────────────────────────────────────────

/// Lower every `OVER (...)` item of a SELECT list into [`WindowField`]s.
///
/// Only a top-level SELECT item may carry a window specification: the frame is
/// evaluated over the whole partitioned row set, which no scalar expression,
/// predicate or aggregate argument can reach.
fn lower_windows(select: &SelectStatement, parameters: &[ParameterDescriptor], ctes: &CteScope) -> Result<Vec<WindowField>> {
    let mut fields = Vec::new();
    for item in &select.items {
        let SelectItem::Expression { expr, alias, .. } = item
        else {
            continue;
        };
        let Expression::FunctionCall { name, args, distinct, window: Some(spec), .. } = expr
        else {
            continue;
        };
        fields.push(lower_window_field(name, args, *distinct, spec, output_name(expr, alias), parameters, ctes)?);
    }
    Ok(fields)
}

/// Lower one window function call.
///
/// The function name decides the arity: the ranking functions take no argument,
/// `COUNT(*)` takes the wildcard, and every other accepted function takes
/// exactly one expression. An unimplemented function is rejected by name rather
/// than being approximated by a nearby ranking function.
fn lower_window_field(
    name: &Identifier,
    args: &[Expression],
    distinct: bool,
    window: &AstWindowSpec,
    output: String,
    parameters: &[ParameterDescriptor],
    ctes: &CteScope,
) -> Result<WindowField> {
    let fn_name = name.name.to_ascii_uppercase();
    let function = match fn_name.as_str() {
        "ROW_NUMBER" => WindowFunction::RowNumber,
        "RANK" => WindowFunction::Rank,
        "DENSE_RANK" => WindowFunction::DenseRank,
        "SUM" => WindowFunction::Sum,
        "COUNT" => WindowFunction::Count,
        "AVG" => WindowFunction::Avg,
        "MIN" => WindowFunction::Min,
        "MAX" => WindowFunction::Max,
        other => {
            return Err(sql_error(format!(
                "window function `{other}` is not supported by YYDB \
                 (supported: `ROW_NUMBER`, `RANK`, `DENSE_RANK`, `SUM`, `COUNT`, `AVG`, `MIN`, `MAX`)"
            )));
        }
    };
    if distinct {
        return Err(sql_error(format!(
            "`{fn_name}(DISTINCT …) OVER (...)` is not supported by YYDB: remove DISTINCT from the window call"
        )));
    }
    let argument = match function {
        WindowFunction::RowNumber | WindowFunction::Rank | WindowFunction::DenseRank => {
            if !args.is_empty() {
                return Err(sql_error(format!("`{fn_name}` takes no arguments, got {}", args.len())));
            }
            None
        }
        WindowFunction::Count if matches!(args, [Expression::Wildcard { .. }]) => None,
        _ => {
            if args.len() != 1 {
                return Err(sql_error(format!("`{fn_name}` requires exactly one argument, got {}", args.len())));
            }
            Some(lower_project_expr(&args[0], parameters, ctes)?)
        }
    };
    Ok(WindowField { name: output, function, argument, spec: lower_window_spec(window, ctes)? })
}

/// Lower an `OVER (...)` clause into the shared [`WindowSpec`].
///
/// `PARTITION BY` and `ORDER BY` accept column references only, which is what
/// the parser already guarantees. An explicit `ROWS` frame lowers into its two
/// bounds, while `RANGE` is rejected: a range frame compares values instead of
/// rows, so reading it as a row offset would silently change which rows an
/// aggregate sees.
fn lower_window_spec(window: &AstWindowSpec, _ctes: &CteScope) -> Result<WindowSpec> {
    let frame = match &window.frame {
        Some(frame) => Some(lower_window_frame(frame)?),
        None => None,
    };
    let mut partition_by = Vec::with_capacity(window.partition_by.len());
    for column in &window.partition_by {
        partition_by.push(ProjectExpr::Scalar { path: split_identifier(column) });
    }
    let mut order_by = Vec::with_capacity(window.order_by.len());
    for item in &window.order_by {
        let WindowOrderItem { column, direction, .. } = item;
        order_by.push(WindowOrderKey {
            expr: ProjectExpr::Scalar { path: split_identifier(column) },
            ascending: !matches!(direction, oak_sql::ast::OrderDirection::Desc),
        });
    }
    Ok(WindowSpec { partition_by, order_by, frame })
}

/// Lower one explicit window frame, which must count rows.
///
/// The single-bound form (`ROWS 2 PRECEDING`) ends at the current row, exactly
/// like `ROWS BETWEEN 2 PRECEDING AND CURRENT ROW`.
fn lower_window_frame(frame: &AstWindowFrame) -> Result<WindowFrame> {
    if matches!(frame.unit, AstWindowFrameUnit::Range) {
        return Err(sql_error(
            "`RANGE` window frames are not supported by YYDB: a range frame compares values instead of rows, \
             so write the frame with `ROWS` between two row offsets",
        ));
    }
    let end = match &frame.end {
        Some(bound) => lower_window_bound(bound),
        None => WindowFrameBound::CurrentRow,
    };
    Ok(WindowFrame { start: lower_window_bound(&frame.start), end })
}

/// Map one written frame bound onto the operator's bound vocabulary.
fn lower_window_bound(bound: &AstWindowFrameBound) -> WindowFrameBound {
    match bound {
        AstWindowFrameBound::UnboundedPreceding => WindowFrameBound::UnboundedPreceding,
        AstWindowFrameBound::Preceding(offset) => WindowFrameBound::Preceding(*offset),
        AstWindowFrameBound::CurrentRow => WindowFrameBound::CurrentRow,
        AstWindowFrameBound::Following(offset) => WindowFrameBound::Following(*offset),
        AstWindowFrameBound::UnboundedFollowing => WindowFrameBound::UnboundedFollowing,
    }
}

/// Split a written identifier into the path components the executor resolves.
fn split_identifier(identifier: &Identifier) -> Vec<String> {
    identifier.name.split('.').map(str::to_string).collect()
}

/// When an Oak identifier is one of the four SQL-standard clock registers
/// written without parentheses (`NOW`, `CURRENT_TIMESTAMP`, `CURRENT_DATE`,
/// `CURRENT_TIME`), lower it to the matching [`ScalarFunction::Now`] variant
/// with no arguments so the executor reads the frozen query-start instant.
fn try_lower_clock_register(identifier: &Identifier) -> Option<Expr> {
    match identifier.name.to_ascii_uppercase().as_str() {
        "NOW" => Some(Expr::Function { function: ScalarFunction::Now, args: Vec::new() }),
        "CURRENT_TIMESTAMP" => Some(Expr::Function { function: ScalarFunction::CurrentTimestamp, args: Vec::new() }),
        "CURRENT_DATE" => Some(Expr::Function { function: ScalarFunction::CurrentDate, args: Vec::new() }),
        "CURRENT_TIME" => Some(Expr::Function { function: ScalarFunction::CurrentTime, args: Vec::new() }),
        _ => None,
    }
}

/// Name of the first window function inside `expr`, if it contains one.
///
/// Subqueries carry their own scope, so they are not searched.
fn first_window_function(expr: &Expression) -> Option<String> {
    match expr {
        Expression::FunctionCall { name, args, window, .. } => {
            if window.is_some() {
                return Some(name.name.to_string());
            }
            args.iter().find_map(first_window_function)
        }
        Expression::Binary { left, right, .. } => first_window_function(left).or_else(|| first_window_function(right)),
        Expression::Unary { expr, .. } | Expression::IsNull { expr, .. } | Expression::IsTruth { expr, .. } => {
            first_window_function(expr)
        }
        Expression::Cast { expr, .. } => first_window_function(expr),
        Expression::Vector { elements, .. } => elements.iter().find_map(first_window_function),
        Expression::InList { expr, list, .. } => {
            first_window_function(expr).or_else(|| list.iter().find_map(first_window_function))
        }
        Expression::Between { expr, low, high, .. } => {
            first_window_function(expr).or_else(|| first_window_function(low)).or_else(|| first_window_function(high))
        }
        Expression::Case { branches, else_branch, .. } => {
            for (when, then) in branches {
                if let Some(name) = first_window_function(when).or_else(|| first_window_function(then)) {
                    return Some(name);
                }
            }
            else_branch.as_ref().and_then(|els| first_window_function(els))
        }
        Expression::Identifier(_)
        | Expression::Literal(_)
        | Expression::Wildcard { .. }
        | Expression::Subquery { .. }
        | Expression::InSubquery { .. }
        | Expression::Exists { .. }
        | Expression::Error { .. } => None,
    }
}

/// Lower one aggregate argument into a [`ProjectExpr`].
///
/// A plain column reference becomes [`ProjectExpr::Scalar`] so the executor can
/// read it straight from the row. Anything else goes through [`lower_expr`], so
/// `SUM(score + 1)` is computed per row instead of being rejected or
/// approximated.
fn lower_project_expr(expr: &Expression, parameters: &[ParameterDescriptor], ctes: &CteScope) -> Result<ProjectExpr> {
    match expr {
        Expression::Identifier(identifier) => {
            // Split qualified names (`table.col`) into path components so the
            // executor can resolve both the full dotted key and the trailing
            // bare column name. A reference to the enclosing statement is marked
            // so execution reads it from that statement's row.
            if let Some(clock) = try_lower_clock_register(identifier) {
                return Ok(ProjectExpr::Computed(Box::new(clock)));
            }
            Ok(ProjectExpr::Scalar { path: reference_path(&identifier.name, ctes) })
        }
        other => Ok(ProjectExpr::Computed(Box::new(lower_expr(other, parameters, ctes)?))),
    }
}

fn lower_sort_key(item: &OrderByItem, parameters: &[ParameterDescriptor], scope: &CteScope) -> Result<SortKey> {
    if let Some(name) = first_window_function(&item.expr) {
        return Err(sql_error(format!(
            "window function `{name}` cannot appear in `ORDER BY`: order by the window column alias instead"
        )));
    }
    let ascending = !matches!(item.direction, oak_sql::ast::OrderDirection::Desc);
    match &item.expr {
        Expression::Identifier(identifier) => Ok(SortKey {
            field: identifier.name.to_string(),
            expr: None,
            ascending,
        }),
        other => Ok(SortKey {
            field: "expr".to_string(),
            expr: Some(lower_expr(other, parameters, scope)?),
            ascending,
        }),
    }
}

fn lower_limit(
    limit: &LimitClause,
    parameters: &[ParameterDescriptor],
    scope: &CteScope,
    ops: &mut Vec<QueryOp>,
) -> Result<()> {
    if let Some(offset) = &limit.offset {
        ops.push(QueryOp::Skip { count: lower_limit_value(offset, parameters, scope)? });
    }
    ops.push(QueryOp::Take { count: lower_limit_value(&limit.limit, parameters, scope)? });
    Ok(())
}

/// Lower one `LIMIT`/`OFFSET` value.
///
/// A column of the enclosing statement is rejected here: the subquery would then
/// change its own row count for every outer row, which per-row re-execution does
/// not model. A correlated subquery is equally rejected. Plain literals and
/// parameter slots stay [`CountExpr::Bound`]; a scalar subquery lowers into
/// [`CountExpr::Expr`]. Anything else (arithmetic on columns, functions over
/// columns) is rejected because there is no meaningful row context to evaluate
/// it against.
fn lower_limit_value(expr: &Expression, parameters: &[ParameterDescriptor], scope: &CteScope) -> Result<CountExpr> {
    match expr {
        Expression::Identifier(identifier) => {
            if scope.reads_outer_row(&identifier.name) {
                return Err(sql_error(format!(
                    "`{}` reads a column of the enclosing statement and cannot set `LIMIT`/`OFFSET`: a correlated subquery runs once per outer row and may not change its own row count",
                    identifier.name
                )));
            }
            Ok(CountExpr::Bound(lower_bound_value(expr, parameters)?))
        }
        Expression::Literal(_) | Expression::Unary { op: UnaryOperator::Minus, .. } => {
            Ok(CountExpr::Bound(lower_bound_value(expr, parameters)?))
        }
        Expression::Subquery { query, .. } => {
            let (ops, correlated) = lower_scalar_subquery(query, parameters, scope)?;
            if correlated {
                return Err(sql_error(
                    "a correlated scalar subquery cannot set `LIMIT`/`OFFSET`: it would re-run and change the row count for every outer row",
                ));
            }
            Ok(CountExpr::Expr(Box::new(Expr::Subquery { ops, correlated: false })))
        }
        other => {
            if expr_has_column_ref(other) {
                return Err(sql_error(
                    "`LIMIT`/`OFFSET` does not support expressions that read table columns: use a literal, parameter or scalar subquery instead",
                ));
            }
            Ok(CountExpr::Expr(Box::new(lower_expr(other, parameters, scope)?)))
        }
    }
}

fn expr_has_column_ref(expr: &Expression) -> bool {
    match expr {
        Expression::Identifier(_) => !matches!(expr, Expression::Identifier(ident) if ident.name.starts_with("__yydb_param_")),
        Expression::Binary { left, right, .. } => expr_has_column_ref(left) || expr_has_column_ref(right),
        Expression::Unary { expr: inner, .. } | Expression::IsNull { expr: inner, .. } | Expression::IsTruth { expr: inner, .. } => expr_has_column_ref(inner),
        Expression::FunctionCall { args, .. } => args.iter().any(expr_has_column_ref),
        Expression::Cast { expr: inner, .. } => expr_has_column_ref(inner),
        Expression::Case { branches, else_branch, .. } => {
            branches.iter().any(|(w, t)| expr_has_column_ref(w) || expr_has_column_ref(t))
                || else_branch.as_ref().is_some_and(|e| expr_has_column_ref(e))
        }
        Expression::Between { expr: inner, low, high, .. } => {
            expr_has_column_ref(inner) || expr_has_column_ref(low) || expr_has_column_ref(high)
        }
        Expression::InList { expr: inner, list, .. } => {
            expr_has_column_ref(inner) || list.iter().any(expr_has_column_ref)
        }
        Expression::InSubquery { expr: inner, .. } => expr_has_column_ref(inner),
        _ => false,
    }
}

/// Returns `None` when the projection is `*` (the scan already yields every
/// stored column). Aggregate function items (`COUNT(...)`, `SUM(...)`) are
/// consumed by the [`QueryOp::Aggregate`] lowering — this function skips them
/// so they do not produce a duplicate Project entry.
fn lower_projection(
    items: &[SelectItem],
    parameters: &[ParameterDescriptor],
    ctes: &CteScope,
) -> Result<Option<Vec<ProjectField>>> {
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
        // Skip aggregate functions — they are consumed by the Aggregate op. A
        // scalar function (`LOWER(email)`, `LENGTH(email)` …) is projected like
        // any other computed expression.
        if let Expression::FunctionCall { name, window: None, .. } = expr
            && is_aggregate_function(name)
        {
            continue;
        }
        let name = output_name(expr, alias);
        // A window call is already computed by the `Window` op that runs before
        // this projection, so the field only re-emits the column it wrote.
        if matches!(expr, Expression::FunctionCall { window: Some(_), .. }) {
            fields.push(ProjectField { name: name.clone(), expr: ProjectExpr::Scalar { path: vec![name.clone()] } });
            continue;
        }
        // Plain identifier → scalar column path. A qualified name arrives as one
        // identifier containing a dot, so it is split like a predicate path:
        // the executor then resolves the qualified key first and falls back to
        // the bare trailing column of an unqualified row source.
        let field_expr = match expr {
            Expression::Identifier(identifier) => {
                if let Some(clock) = try_lower_clock_register(identifier) {
                    ProjectExpr::Computed(Box::new(clock))
                }
                else {
                    ProjectExpr::Scalar { path: reference_path(&identifier.name, ctes) }
                }
            }
            // Arbitrary lowerable expression (CASE, CAST, nested CASE inside CASE,
            // etc.) → generic Computed wrapper around our Expr IR.
            other => ProjectExpr::Computed(Box::new(lower_expr(other, parameters, ctes)?)),
        };
        fields.push(ProjectField { name, expr: field_expr });
    }
    Ok(Some(fields))
}

// ─── predicates ──────────────────────────────────────────────────────────

fn lower_predicate(expr: &Expression, parameters: &[ParameterDescriptor], ctes: &CteScope) -> Result<Pred> {
    if let Some(name) = first_window_function(expr) {
        return Err(sql_error(format!(
            "window function `{name}` cannot appear in a predicate (`WHERE`, `JOIN ON`, `HAVING` or `ON CONFLICT WHERE`)"
        )));
    }
    match expr {
        Expression::Literal(Literal::Boolean(value, _)) => Ok(if *value { Pred::True } else { Pred::False }),
        Expression::Binary { left, op: BinaryOperator::And, right, .. } => Ok(Pred::And(
            Box::new(lower_predicate(left, parameters, ctes)?),
            Box::new(lower_predicate(right, parameters, ctes)?),
        )),
        Expression::Binary { left, op: BinaryOperator::Or, right, .. } => Ok(Pred::Or(
            Box::new(lower_predicate(left, parameters, ctes)?),
            Box::new(lower_predicate(right, parameters, ctes)?),
        )),
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
            // A scalar subquery on the left of the comparison is mirrored so the
            // field stays on the left-hand side, flipping the operator to match.
            if let Expression::Subquery { query, .. } = left.as_ref() {
                let (ops, correlated) = lower_scalar_subquery(query, parameters, ctes)?;
                return Ok(Pred::FieldCmpSubquery { path: column_path(right, ctes)?, op: flip_cmp(cmp), ops, correlated });
            }
            // A non-identifier left-hand side (function call, arithmetic, CASE,
            // CAST, parameter, …) lowers to `ExprCmp` so the executor evaluates
            // the expression per row and compares the result with SQL three-valued
            // NULL semantics. Subquery on the right still mirrors onto
            // `FieldCmpSubquery` with a path on the left — but `ExprCmp` cannot
            // carry a subquery on the right, so reject that shape explicitly.
            if !matches!(left.as_ref(), Expression::Identifier(_)) {
                if let Expression::Subquery { .. } = right.as_ref() {
                    return Err(sql_error("comparing a scalar expression with a subquery on the right is not supported"));
                }
                let expr = lower_expr(left, parameters, ctes)?;
                let value = lower_bound_value(right, parameters)?;
                return Ok(Pred::ExprCmp { expr: Box::new(expr), op: cmp, value });
            }
            let left_path = column_path(left, ctes)?;
            if let Expression::Subquery { query, .. } = right.as_ref() {
                let (ops, correlated) = lower_scalar_subquery(query, parameters, ctes)?;
                return Ok(Pred::FieldCmpSubquery { path: left_path, op: cmp, ops, correlated });
            }
            // Detect column-vs-column comparisons (JOIN ON). If the right side
            // is a plain identifier that does NOT look like a rewritten param,
            // treat it as a column path instead of a bound value.
            if let Expression::Identifier(right_ident) = right.as_ref() {
                let right_name = right_ident.name.as_ref();
                if !right_name.starts_with("__yydb_param_") {
                    let right_path = reference_path(right_name, ctes);
                    return Ok(Pred::FieldCmpField { left_path, op: cmp, right_path });
                }
            }
            let value = lower_bound_value(right, parameters)?;
            // A plain column comparison goes through `ExprCmp` so a `NULL` on
            // either side is SQL unknown. `FieldCmp` keeps VOS ordinary equality
            // and the decided `IS [NOT] TRUE/FALSE` tests.
            Ok(Pred::ExprCmp { expr: Box::new(Expr::Column { path: left_path }), op: cmp, value })
        }
        Expression::Unary { op: UnaryOperator::Not, expr, .. } => {
            Ok(Pred::Not(Box::new(lower_predicate(expr, parameters, ctes)?)))
        }
        Expression::IsNull { expr, negated, .. } => {
            // `IS [NOT] NULL` is decided for every operand shape, including a
            // bare column, so it never lowers to a comparison against `NULL`.
            let lowered = lower_expr(expr, parameters, ctes)?;
            Ok(Pred::ExprIsNull { expr: Box::new(lowered), negated: *negated })
        }
        Expression::IsTruth { expr, value, negated, .. } => {
            // `IS [NOT] TRUE/FALSE` is decided as well: a `NULL` operand is
            // FALSE for the positive forms instead of unknown.
            let lowered = lower_expr(expr, parameters, ctes)?;
            Ok(Pred::ExprIsTruth { expr: Box::new(lowered), value: *value, negated: *negated })
        }
        Expression::InList { expr, list, negated, .. } => {
            let lowered = lower_expr(expr, parameters, ctes)?;
            let mut values = Vec::with_capacity(list.len());
            for item in list {
                values.push(lower_bound_value(item, parameters)?);
            }
            if values.is_empty() {
                return Err(sql_error("IN requires at least one value"));
            }
            Ok(Pred::ExprInList { expr: Box::new(lowered), list: values, negated: *negated })
        }
        Expression::Between { expr, low, high, negated, .. } => {
            let lowered = lower_expr(expr, parameters, ctes)?;
            Ok(Pred::ExprBetween {
                expr: Box::new(lowered),
                low: lower_bound_value(low, parameters)?,
                high: lower_bound_value(high, parameters)?,
                negated: *negated,
            })
        }
        Expression::Identifier(identifier) => {
            Ok(Pred::FieldBool { path: reference_path(&identifier.name, ctes), value: BoundValue::Literal(Value::Bool(true)) })
        }
        Expression::Exists { query, negated, .. } => {
            let (ops, correlated) = lower_select(query, parameters, ctes)?;
            Ok(Pred::Exists { ops, negated: *negated, correlated })
        }
        Expression::InSubquery { expr, query, negated, .. } => {
            // The test compares one value with one subquery column, so a wider
            // projection is rejected instead of comparing only its first column.
            if let Some(columns) = select_output_names(&query.items)?
                && columns.len() != 1
            {
                return Err(sql_error(format!(
                    "`IN (SELECT ...)` requires a single-column subquery, but this one produces {} columns",
                    columns.len()
                )));
            }
            let (ops, correlated) = lower_select(query, parameters, ctes)?;
            if let Ok(path) = column_path(expr, ctes) {
                Ok(Pred::InSubquery { path, ops, negated: *negated, correlated })
            }
            else {
                let lowered = lower_expr(expr, parameters, ctes)?;
                Ok(Pred::ExprInSubquery { expr: Box::new(lowered), ops, negated: *negated, correlated })
            }
        }
        Expression::Subquery { query, .. } => {
            let (ops, correlated) = lower_scalar_subquery(query, parameters, ctes)?;
            Ok(Pred::SubqueryAsBool { ops, correlated })
        }
        _ => Err(sql_error("unsupported SQL predicate shape")),
    }
}

/// Mirror a comparison operator, used when a scalar subquery is rewritten from
/// the left-hand side of a comparison onto the right-hand side.
fn flip_cmp(op: CmpOp) -> CmpOp {
    match op {
        CmpOp::Eq => CmpOp::Eq,
        CmpOp::Ne => CmpOp::Ne,
        CmpOp::Lt => CmpOp::Gt,
        CmpOp::Le => CmpOp::Ge,
        CmpOp::Gt => CmpOp::Lt,
        CmpOp::Ge => CmpOp::Le,
    }
}

fn column_path(expr: &Expression, scope: &CteScope) -> Result<Vec<String>> {
    let Expression::Identifier(identifier) = expr
    else {
        return Err(sql_error("predicates support plain column references only"));
    };
    // Oak-sql represents qualified names (`table.col`) as a single identifier
    // whose name contains a dot. Split it so the executor can resolve both
    // the full dotted key (right-table columns with prefix) and the trailing
    // bare column (left-table columns without prefix).
    Ok(reference_path(&identifier.name, scope))
}

/// Split one written column reference into the path the executor resolves.
///
/// A reference to the enclosing statement carries [`OUTER_ROW_MARKER`] as its
/// first component, so execution reads it from the enclosing row rather than
/// from the row under test.
fn reference_path(name: &str, scope: &CteScope) -> Vec<String> {
    let mut path: Vec<String> = name.split('.').map(str::to_string).collect();
    if scope.reads_outer_row(name) {
        path.insert(0, OUTER_ROW_MARKER.to_string());
    }
    path
}

fn lower_bound_value(expr: &Expression, parameters: &[ParameterDescriptor]) -> Result<BoundValue> {
    match expr {
        Expression::Literal(Literal::Null(_)) => Ok(BoundValue::Literal(Value::Null)),
        Expression::Literal(Literal::Boolean(value, _)) => Ok(BoundValue::Literal(Value::Bool(*value))),
        Expression::Literal(Literal::String(text, _)) => Ok(BoundValue::Literal(Value::Text(text.to_string()))),
        Expression::Literal(Literal::Number(text, _)) => Ok(BoundValue::Literal(numeric_literal(text)?)),
        Expression::Unary { op: UnaryOperator::Minus, expr, .. } => {
            let Expression::Literal(Literal::Number(text, _)) = expr.as_ref()
            else {
                return Err(sql_error("unary minus supports numeric literals only"));
            };
            Ok(BoundValue::Literal(negate_number(numeric_literal(text)?)))
        }
        Expression::Identifier(identifier) => name_to_slot(&identifier.name, parameters),
        // A `CAST` over a literal is a constant, so it folds into the literal the
        // cast produces. Folding here keeps a comparison against
        // `CAST('…' AS BLOB)` identical to the same value written as a parameter.
        // A `CAST` over a placeholder is not folded: YYDB decides that shape at
        // execution time, and a comparison value is baked into the plan, so the
        // open form is rejected instead of being silently dropped.
        Expression::Cast { expr: inner, target, .. } => {
            let BoundValue::Literal(value) = lower_bound_value(inner, parameters)?
            else {
                return Err(sql_error("CAST over a parameter cannot be used as a comparison value"));
            };
            Ok(BoundValue::Literal(super::exec::eval_cast(&value, target)?))
        }
        _ => Err(sql_error("unsupported SQL value expression")),
    }
}

/// Parse one SQL numeric literal into the exact scalar it denotes.
///
/// A literal that writes a fractional part or an exponent (`3.14`, `1e3`) is
/// REAL and becomes [`Value::F64`]. A plain run of digits is an integer and
/// becomes [`Value::I64`]. Both are parsed exactly as written: an integer
/// literal outside the signed 64-bit range is rejected instead of being rounded
/// into REAL, and a REAL literal that overflows the finite `f64` range is
/// rejected instead of becoming an infinity.
///
/// A REAL value is a transient expression type: YYDB persists integer, bool,
/// text, uuid, date, time, timestamp and decimal columns only, so storing a
/// REAL literal is rejected by the catalog-directed coercion.
fn numeric_literal(text: &str) -> Result<Value> {
    if text.contains('.') || text.contains('e') || text.contains('E') {
        let value: f64 = text.parse().map_err(|_| sql_error(format!("`{text}` is not a valid REAL literal")))?;
        if !value.is_finite() {
            return Err(sql_error(format!("REAL literal `{text}` is outside the finite `f64` range")));
        }
        Ok(Value::F64(value))
    }
    else {
        let value: i64 =
            text.parse().map_err(|_| sql_error(format!("integer literal `{text}` is outside the signed 64-bit range")))?;
        Ok(Value::I64(value))
    }
}

/// Apply unary minus to a literal already typed by [`numeric_literal`].
fn negate_number(value: Value) -> Value {
    match value {
        Value::I64(number) => Value::I64(-number),
        Value::F64(number) => Value::F64(-number),
        other => other,
    }
}

/// Lower an arbitrary SQL expression into the backend-neutral [`Expr`] IR.
///
/// The accepted surface is primitive terminals (identifiers → columns, literals
/// and parameter bindings → bound values), scalar function calls (recursively
/// lowered into [`Expr::Function`]), arithmetic and comparison/logical
/// [`Expression::Binary`] nodes, and the compound nodes [`Expression::Case`] /
/// [`Expression::Cast`]. A scalar subquery stays a nested pipeline in
/// [`Expr::Subquery`] and is resolved at execution time. Everything else
/// (`EXISTS`, `IN` …) is rejected here; callers decide where this IR is allowed —
/// today SELECT projection, aggregate arguments and `RETURNING` use it.
fn lower_expr(expr: &Expression, parameters: &[ParameterDescriptor], ctes: &CteScope) -> Result<Expr> {
    match expr {
        // Bare identifiers are column references — except for the four SQL
        // standard clock registers that Oak keeps as plain names rather than
        // function calls: `CURRENT_TIMESTAMP`, `CURRENT_DATE`, `CURRENT_TIME`,
        // and also `NOW` written without parentheses. Lower them to the clock
        // scalar functions so the executor can freeze them at query start.
        Expression::Identifier(identifier) => {
            // A rewritten placeholder (`__yydb_param_<index>`) is a bound value,
            // never a column: `CAST(? AS BLOB)` casts the bound value instead of
            // reading a column whose name happens to look like a placeholder.
            if identifier.name.starts_with(PARAM_PREFIX) {
                return Ok(Expr::Bound(name_to_slot(&identifier.name, parameters)?));
            }
            if let Some(clock) = try_lower_clock_register(identifier) {
                return Ok(clock);
            }
            Ok(Expr::Column { path: reference_path(&identifier.name, ctes) })
        }
        Expression::Literal(_) | Expression::Unary { op: UnaryOperator::Minus, .. } => {
            Ok(Expr::Bound(lower_bound_value(expr, parameters)?))
        }
        Expression::Case { branches, else_branch, .. } => {
            let mut lowered_branches = Vec::with_capacity(branches.len());
            for (when, then) in branches {
                lowered_branches
                    .push((Box::new(lower_expr(when, parameters, ctes)?), Box::new(lower_expr(then, parameters, ctes)?)));
            }
            let lowered_else = else_branch.as_ref().map(|els| lower_expr(els, parameters, ctes)).transpose()?.map(Box::new);
            Ok(Expr::Case { branches: lowered_branches, else_branch: lowered_else })
        }
        Expression::Cast { expr: inner, target, .. } => {
            Ok(Expr::Cast { expr: Box::new(lower_expr(inner, parameters, ctes)?), target: target.to_ascii_uppercase() })
        }
        Expression::Binary { left, op, right, .. } => {
            let bin_op = match op {
                BinaryOperator::Plus => BinaryOp::Add,
                BinaryOperator::Minus => BinaryOp::Sub,
                BinaryOperator::Star => BinaryOp::Mul,
                BinaryOperator::Slash => BinaryOp::Div,
                BinaryOperator::Percent => BinaryOp::Rem,
                BinaryOperator::Equal => BinaryOp::Eq,
                BinaryOperator::NotEqual => BinaryOp::Ne,
                BinaryOperator::Less => BinaryOp::Lt,
                BinaryOperator::LessEqual => BinaryOp::Le,
                BinaryOperator::Greater => BinaryOp::Gt,
                BinaryOperator::GreaterEqual => BinaryOp::Ge,
                BinaryOperator::And => BinaryOp::And,
                BinaryOperator::Or => BinaryOp::Or,
                other => {
                    return Err(sql_error(format!("binary operator `{other:?}` is not supported in projection expressions")));
                }
            };
            Ok(Expr::Binary {
                op: bin_op,
                left: Box::new(lower_expr(left, parameters, ctes)?),
                right: Box::new(lower_expr(right, parameters, ctes)?),
            })
        }
        Expression::Unary { op: UnaryOperator::Not, expr, .. } => Ok(Expr::Not(Box::new(lower_expr(expr, parameters, ctes)?))),
        Expression::Subquery { query, .. } => {
            let (ops, correlated) = lower_scalar_subquery(query, parameters, ctes)?;
            Ok(Expr::Subquery { ops, correlated })
        }
        Expression::FunctionCall { name, args, window, .. } => {
            // A window value is not a scalar: it depends on the whole partitioned
            // row set, so it may only appear as its own SELECT item.
            if window.is_some() {
                return Err(sql_error(format!("window function `{}` is only supported as a top-level SELECT item", name.name)));
            }
            lower_scalar_function(name, args, parameters, ctes)
        }
        Expression::IsNull { .. }
        | Expression::IsTruth { .. }
        | Expression::InList { .. }
        | Expression::Between { .. }
        | Expression::InSubquery { .. }
        | Expression::Exists { .. }
        | Expression::Unary { op: _, expr: _, .. }
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
                Expression::Exists { .. } => "EXISTS (subquery)",
                Expression::Unary { .. } => "unary",
                Expression::Wildcard { .. } => "*",
                Expression::Vector { .. } => "vector",
                Expression::Error { .. } => "error node",
                _ => "other",
            }
        ))),
    }
}

/// Lower one scalar function call into [`Expr::Function`].
///
/// Argument counts are checked here, at plan time, so a malformed call is
/// rejected while preparing instead of failing on the first row. `NULL`
/// propagation and argument types are execution concerns because lowering has
/// no column types. The accepted set is the text functions (`LOWER`, `UPPER`,
/// `LENGTH`, `CHAR_LENGTH`, `SUBSTR`, `TRIM`, `REPLACE`, `CONCAT`) and the date
/// part extractors (`YEAR`, `MONTH`, `DAY`), whose argument must be a `date` or
/// `datetime` column rather than text that the executor would have to guess a
/// format for. `COALESCE` takes any number of arguments because its result kind
/// is decided at execution time from the first non-`NULL` argument.
fn lower_scalar_function(
    name: &Identifier,
    args: &[Expression],
    parameters: &[ParameterDescriptor],
    ctes: &CteScope,
) -> Result<Expr> {
    let fn_name = name.name.to_ascii_uppercase();
    // Aggregates are consumed by the Aggregate op only as a top-level SELECT
    // item, so one reached through another call is a nested aggregate: `SUM(AVG(x))`
    // has no per-row value to aggregate and is rejected instead of being read as
    // an unknown scalar function.
    if is_aggregate_function(name) {
        return Err(sql_error(format!("aggregate function `{fn_name}` cannot be nested inside another function call")));
    }
    let (function, arity_ok, expected) = match fn_name.as_str() {
        "LOWER" => (ScalarFunction::Lower, args.len() == 1, "exactly one argument"),
        "UPPER" => (ScalarFunction::Upper, args.len() == 1, "exactly one argument"),
        "LENGTH" => (ScalarFunction::Length, args.len() == 1, "exactly one argument"),
        "CHAR_LENGTH" => (ScalarFunction::CharLength, args.len() == 1, "exactly one argument"),
        "TRIM" => (ScalarFunction::Trim, args.len() == 1, "exactly one argument"),
        "REPLACE" => (ScalarFunction::Replace, args.len() == 3, "exactly three arguments"),
        "SUBSTR" => (ScalarFunction::Substr, args.len() == 2 || args.len() == 3, "two or three arguments"),
        "CONCAT" => (ScalarFunction::Concat, args.len() >= 2, "two or more arguments"),
        "YEAR" => (ScalarFunction::Year, args.len() == 1, "exactly one argument"),
        "MONTH" => (ScalarFunction::Month, args.len() == 1, "exactly one argument"),
        "DAY" => (ScalarFunction::Day, args.len() == 1, "exactly one argument"),
        "COALESCE" => (ScalarFunction::Coalesce, !args.is_empty(), "at least one argument"),
        "NOW" | "CURRENT_TIMESTAMP" => {
            if !args.is_empty() {
                return Err(sql_error(format!("`{fn_name}` takes no arguments, got {}", args.len())));
            }
            // Clock functions freeze at execution time, not lowering time. The
            // executor reads a thread-local query start instant so repeated
            // calls within one query (or across rows of a projection) yield
            // the same value.
            let function = match fn_name.as_str() {
                "NOW" => ScalarFunction::Now,
                _ => ScalarFunction::CurrentTimestamp,
            };
            return Ok(Expr::Function { function, args: Vec::new() });
        }
        "CURRENT_DATE" => {
            if !args.is_empty() {
                return Err(sql_error(format!("`{fn_name}` takes no arguments, got {}", args.len())));
            }
            return Ok(Expr::Function { function: ScalarFunction::CurrentDate, args: Vec::new() });
        }
        "CURRENT_TIME" => {
            if !args.is_empty() {
                return Err(sql_error(format!("`{fn_name}` takes no arguments, got {}", args.len())));
            }
            return Ok(Expr::Function { function: ScalarFunction::CurrentTime, args: Vec::new() });
        }
        other => return Err(sql_error(format!("scalar function `{other}` is not supported by YYDB"))),
    };
    if !arity_ok {
        return Err(sql_error(format!("`{fn_name}` requires {expected}, got {}", args.len())));
    }
    let mut lowered = Vec::with_capacity(args.len());
    for arg in args {
        lowered.push(lower_expr(arg, parameters, ctes)?);
    }
    Ok(Expr::Function { function, args: lowered })
}

/// Lower a `(SELECT ...)` used as a value.
///
/// The subquery must project exactly one column: a wider projection is rejected
/// here instead of silently yielding only its first column. `SELECT *` inside
/// the subquery has no known width until execution, which re-checks the
/// executed row shape. The second element reports whether the subquery reads the
/// enclosing statement's row.
fn lower_scalar_subquery(
    query: &SelectStatement,
    parameters: &[ParameterDescriptor],
    ctes: &CteScope,
) -> Result<(Vec<QueryOp>, bool)> {
    let width = match query.items.as_slice() {
        [SelectItem::Star { .. }] => None,
        items => Some(items.len()),
    };
    if let Some(width) = width
        && width != 1
    {
        return Err(sql_error(format!(
            "a scalar subquery must produce a single column, but this one produces {width} columns"
        )));
    }
    lower_select(query, parameters, ctes)
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
        // Write assignments are evaluated against the row being written, so a
        // subquery has no enclosing statement to read and is rejected by name.
        Expression::Subquery { .. } | Expression::InSubquery { .. } | Expression::Exists { .. } => Err(sql_error(
            "a subquery is not supported in a write assignment: an `INSERT`, `UPDATE` or `DELETE` assignment cannot evaluate one",
        )),
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
fn lower_returning(insert: &InsertStatement, parameters: &[ParameterDescriptor]) -> Result<Option<ReturningProjection>> {
    let Some(clause) = &insert.returning
    else {
        return Ok(None);
    };
    if clause.items.is_empty() {
        return Err(sql_error("RETURNING requires at least one item"));
    }
    if clause
        .items
        .iter()
        .any(|item| matches!(item, SelectItem::Expression { expr, .. } if first_window_function(expr).is_some()))
    {
        return Err(sql_error("RETURNING does not support window functions"));
    }
    if clause.items.iter().any(|item| {
        matches!(item, SelectItem::Expression { expr: Expression::FunctionCall { name, .. }, .. } if is_aggregate_function(name))
    }) {
        return Err(sql_error(
            "RETURNING supports columns and scalar expressions: aggregate functions are not supported",
        ));
    }
    Ok(match lower_projection(&clause.items, parameters, &CteScope::new())? {
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
            predicate: selection.as_ref().map(|expr| lower_conflict_predicate(expr, parameters)).transpose()?,
        },
    };
    Ok(Some(ConflictClause { columns, action }))
}

/// Lower the `SET` list of `ON CONFLICT … DO UPDATE`.
///
/// The assignments reuse the update write-expression surface, so an expression
/// the executor cannot evaluate is rejected instead of being approximated.
fn lower_conflict_assignments(assignments: &[Assignment], parameters: &[ParameterDescriptor]) -> Result<Vec<WriteAssignment>> {
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
    let predicate = lower_predicate(expr, parameters, &CteScope::new())?;
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
    fn check_expr(expr: &Expr) -> Result<()> {
        match expr {
            Expr::Column { path } => check(path),
            Expr::Bound(_) => Ok(()),
            Expr::Case { branches, else_branch } => {
                for (when, then) in branches {
                    check_expr(when)?;
                    check_expr(then)?;
                }
                match else_branch {
                    Some(expr) => check_expr(expr),
                    None => Ok(()),
                }
            }
            Expr::Cast { expr, .. } => check_expr(expr),
            Expr::Binary { left, right, .. } => {
                check_expr(left)?;
                check_expr(right)
            }
            Expr::Not(inner) => check_expr(inner),
            Expr::Function { args, .. } => {
                for arg in args {
                    check_expr(arg)?;
                }
                Ok(())
            }
            // A subquery has its own scope, checked when it is lowered.
            Expr::Subquery { .. } => Ok(()),
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
        // The subquery has its own scope, checked when it is lowered. The tested
        // expression still belongs to this statement, so its path is checked.
        Pred::Exists { .. } => Ok(()),
        Pred::InSubquery { path, .. } | Pred::InSet { path, .. } => check(path),
        // The subquery has its own scope, checked when it is lowered. The
        // compared field still belongs to this statement.
        Pred::FieldCmpSubquery { path, .. } | Pred::FieldCmpScalar { path, .. } => check(path),
        // Every expression-bearing predicate carries its column paths inside
        // `Expr`, so the whole expression tree is checked here.
        Pred::ExprCmp { expr, .. }
        | Pred::ExprIsNull { expr, .. }
        | Pred::ExprIsTruth { expr, .. }
        | Pred::ExprBetween { expr, .. }
        | Pred::ExprInList { expr, .. } => check_expr(expr),
        // Subquery has its own scope checked at lowering time; the tested expr
        // was already validated while being lowered.
        Pred::ExprInSubquery { .. } | Pred::ExprInSet { .. } => Ok(()),
        // A bare subquery-as-bool has its own scope, checked when lowered.
        Pred::SubqueryAsBool { .. } => Ok(()),
    }
}

/// Normalise one written conflict target: the bare column, or a table-qualified
/// reference that must name the statement's own target table.
fn conflict_target_column(written: &str, table: &str) -> Result<String> {
    let parts: Vec<&str> = written.split('.').collect();
    match parts.as_slice() {
        [column] => Ok((*column).to_string()),
        [prefix, column] if *prefix == table => Ok((*column).to_string()),
        _ => Err(sql_error(format!("ON CONFLICT target `{written}` must be a column of the insert target `{table}`"))),
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
    if select
        .items
        .iter()
        .any(|item| matches!(item, SelectItem::Expression { expr: Expression::FunctionCall { name, .. }, .. } if is_aggregate_function(name)))
    {
        return Err(sql_error("INSERT ... SELECT source must be a plain row projection: aggregate functions are not supported yet"));
    }

    let outputs = select_output_names(&select.items)?;
    let source = lower_select(select, parameters, &CteScope::new())?.0;
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
            Expression::FunctionCall { name, .. } => name.name.to_ascii_lowercase(),
            _ => "expr".to_string(),
        },
        |alias| alias.name.to_string(),
    )
}

/// Whether a function call is one of the aggregates the [`QueryOp::Aggregate`]
/// lowering consumes.
///
/// Every other call is a scalar function, which flows through [`lower_expr`]
/// into the projection or aggregate argument instead.
fn is_aggregate_function(name: &Identifier) -> bool {
    matches!(name.name.to_ascii_uppercase().as_str(), "COUNT" | "SUM" | "AVG" | "MIN" | "MAX")
}

/// Output column names of a SELECT list, in projection order.
///
/// Returns `None` for `SELECT *`, whose columns are only known once the scanned
/// table is resolved against the catalog.
fn select_output_names(items: &[SelectItem]) -> Result<Option<Vec<String>>> {
    if items.len() == 1 && matches!(items[0], SelectItem::Star { .. }) {
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
    collect_read_tables(source, &mut tables);
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
    let predicate = update.selection.as_ref().map(|expr| lower_predicate(expr, parameters, &CteScope::new())).transpose()?;
    Ok(WriteOp::Update { table: table_name(&update.table_name), assignments, predicate })
}

fn lower_delete(delete: &DeleteStatement, parameters: &[ParameterDescriptor]) -> Result<WriteOp> {
    let predicate = delete.selection.as_ref().map(|expr| lower_predicate(expr, parameters, &CteScope::new())).transpose()?;
    Ok(WriteOp::Delete { table: table_name(&delete.table_name), predicate })
}

fn table_name(table: &TableName) -> String {
    table.name.name.to_string()
}

/// One row source of a `FROM`/`JOIN` clause together with the name its columns
/// are referenced through.
#[derive(Debug, Clone)]
enum RowSource {
    /// A stored table.
    Table {
        /// Storage table the source scans.
        table: String,
        /// Alias written after the table, if any.
        alias: Option<String>,
    },
    /// A derived table: an already lowered sub-pipeline. It always carries an
    /// alias, so its columns are always qualified.
    Derived {
        /// Alias the statement references the derived columns through.
        alias: String,
        /// Inner pipeline producing the derived rows.
        ops: Vec<QueryOp>,
    },
    /// A `WITH` name: an already lowered pipeline referenced like a table, so it
    /// is qualified only when the statement writes an alias after it.
    Cte {
        /// Name the clause gave the pipeline.
        name: String,
        /// Alias written after the name, if any.
        alias: Option<String>,
        /// Pipeline the clause lowered.
        ops: Vec<QueryOp>,
    },
}

impl RowSource {
    /// Name the statement references this source's columns with: the alias when
    /// one was written, otherwise the table or `WITH` name.
    fn reference(&self) -> String {
        match self {
            RowSource::Table { table, alias } => alias.clone().unwrap_or_else(|| table.clone()),
            RowSource::Derived { alias, .. } => alias.clone(),
            RowSource::Cte { name, alias, .. } => alias.clone().unwrap_or_else(|| name.clone()),
        }
    }

    /// Whether this source is addressed through an alias, which makes bare
    /// column references ambiguous.
    fn is_aliased(&self) -> bool {
        match self {
            RowSource::Table { alias, .. } => alias.is_some(),
            RowSource::Derived { .. } => true,
            RowSource::Cte { alias, .. } => alias.is_some(),
        }
    }
}

/// The ops a nested source contributes inline: its pipeline without the
/// `Collect` that only marks the end of its own statement.
fn nested_ops(ops: &[QueryOp]) -> Vec<QueryOp> {
    let mut ops = ops.to_vec();
    if matches!(ops.last(), Some(QueryOp::Collect)) {
        ops.pop();
    }
    ops
}

/// Resolve one `FROM`/`JOIN` row source.
///
/// A derived table keeps its own `SELECT` scope, so its sub-pipeline is lowered
/// here and joins the outer pipeline as a nested operator list. It needs an
/// alias because that alias is the only name the outer statement can reference
/// its columns with. A `WITH` name resolves to the pipeline its clause already
/// lowered and is referenced like a table.
fn row_source(source: &TableSource, parameters: &[ParameterDescriptor], ctes: &CteScope) -> Result<RowSource> {
    match source {
        TableSource::Table(table) => {
            let name = table.name.name.to_string();
            let alias = table.alias.as_ref().map(|alias| alias.name.to_string());
            if let Some(ops) = ctes.pipelines.get(&name) {
                return Ok(RowSource::Cte { name, alias, ops: ops.clone() });
            }
            Ok(RowSource::Table { table: name, alias })
        }
        TableSource::Derived { query, alias, .. } => {
            let alias = alias
                .as_ref()
                .map(|alias| alias.name.to_string())
                .ok_or_else(|| sql_error("a derived table (`FROM (SELECT ...)`) requires an alias"))?;
            // A derived table is a statement of its own: it has no row of the
            // enclosing statement to read, so a correlated reference inside it
            // names no row source and is rejected when the body is checked.
            let mut body = ctes.clone();
            body.frames.clear();
            let (ops, _) = lower_select(query, parameters, &body)?;
            Ok(RowSource::Derived { alias, ops })
        }
    }
}

/// Append every table a read pipeline scans, descending into derived tables.
fn collect_read_tables(ops: &[QueryOp], tables: &mut Vec<String>) {
    for op in ops {
        match op {
            QueryOp::Scan { table } => push_table(tables, table),
            QueryOp::Join { right, .. } => push_table(tables, right),
            QueryOp::JoinSubquery { right, .. } => collect_read_tables(right, tables),
            QueryOp::SetOperation { left, right, .. } => {
                collect_read_tables(left, tables);
                collect_read_tables(right, tables);
            }
            _ => {}
        }
    }
}

fn push_table(tables: &mut Vec<String>, table: &str) {
    if !tables.iter().any(|existing| existing == table) {
        tables.push(table.to_string());
    }
}

/// Where inside a statement a column reference appears.
///
/// A reference a statement evaluates per row may read the enclosing statement's
/// row, which is exactly what makes the subquery correlated. A reference that
/// decides the whole row set instead — `GROUP BY`, `ORDER BY`, a window clause —
/// cannot, because it is not evaluated once per outer row.
#[derive(Clone, Copy)]
enum RefSite {
    /// Row-level expression: `WHERE`, `ON`, a projection or an argument.
    Row,
    /// Whole-result expression, tagged with the clause it appears in.
    WholeResult(&'static str),
}

/// Check every column reference of one statement against its own row sources and
/// the statements that enclose it.
///
/// A reference to an enclosing statement makes the subquery correlated: it is
/// re-executed once per row of that statement, which is the only correlation
/// execution models. Only one level is supported, and only where the outer
/// statement still has a single row to hand down, so the same walk rejects a
/// grandparent reference and a reference to a statement that aggregates its rows.
///
/// When the statement aliases a source, every reference must additionally be
/// qualified: a bare `id` has no single owner and would silently read whichever
/// occurrence happens to carry it.
fn check_select_references(select: &SelectStatement, scope: &CteScope) -> Result<()> {
    for item in &select.items {
        if let SelectItem::Expression { expr, .. } = item {
            check_reference(expr, scope, RefSite::Row)?;
        }
    }
    for join in &select.joins {
        if let Some(on) = &join.on {
            check_reference(on, scope, RefSite::Row)?;
        }
    }
    if let Some(expr) = &select.expr {
        check_reference(expr, scope, RefSite::Row)?;
    }
    if let Some(group_by) = &select.group_by {
        for item in &group_by.items {
            match item {
                GroupByItem::Column(identifier) => check_identifier(identifier, scope, RefSite::WholeResult("GROUP BY"))?,
                GroupByItem::Expression(expr) => check_reference(expr, scope, RefSite::WholeResult("GROUP BY"))?,
                GroupByItem::Position { .. } => {}
                GroupByItem::GroupingSets(inner) => {
                    for set in inner {
                        for sub in set {
                            match sub {
                                GroupByItem::Column(identifier) => check_identifier(identifier, scope, RefSite::WholeResult("GROUP BY"))?,
                                GroupByItem::Expression(expr) => check_reference(expr, scope, RefSite::WholeResult("GROUP BY"))?,
                                _ => {}
                            }
                        }
                    }
                }
                GroupByItem::Rollup(inner) | GroupByItem::Cube(inner) => {
                    for sub in inner {
                        match sub {
                            GroupByItem::Column(identifier) => check_identifier(identifier, scope, RefSite::WholeResult("GROUP BY"))?,
                            GroupByItem::Expression(expr) => check_reference(expr, scope, RefSite::WholeResult("GROUP BY"))?,
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    if let Some(having) = &select.having {
        check_reference(&having.condition, scope, RefSite::Row)?;
    }
    if let Some(order_by) = &select.order_by {
        for item in &order_by.items {
            check_reference(&item.expr, scope, RefSite::WholeResult("ORDER BY"))?;
        }
    }
    Ok(())
}

/// Walk one expression, checking every column reference it contains.
///
/// Subqueries carry their own `FROM` scope, so their columns are checked when
/// the subquery itself is lowered.
fn check_reference(expr: &Expression, scope: &CteScope, site: RefSite) -> Result<()> {
    match expr {
        Expression::Identifier(identifier) => check_identifier(identifier, scope, site),
        Expression::Binary { left, right, .. } => {
            check_reference(left, scope, site)?;
            check_reference(right, scope, site)
        }
        Expression::Unary { expr, .. } | Expression::IsNull { expr, .. } | Expression::IsTruth { expr, .. } => {
            check_reference(expr, scope, site)
        }
        Expression::Cast { expr, .. } => check_reference(expr, scope, site),
        Expression::FunctionCall { args, window, .. } => {
            for arg in args {
                check_reference(arg, scope, site)?;
            }
            // The window clause reads the same row, so its columns must belong to
            // a source of this statement like any other reference. It partitions
            // the whole row set, so a correlated column cannot name one row.
            if let Some(window) = window {
                let window_site = RefSite::WholeResult("OVER (...)");
                for column in &window.partition_by {
                    check_identifier(column, scope, window_site)?;
                }
                for item in &window.order_by {
                    check_identifier(&item.column, scope, window_site)?;
                }
            }
            Ok(())
        }
        Expression::Vector { elements: args, .. } => {
            for arg in args {
                check_reference(arg, scope, site)?;
            }
            Ok(())
        }
        Expression::Between { expr, low, high, .. } => {
            check_reference(expr, scope, site)?;
            check_reference(low, scope, site)?;
            check_reference(high, scope, site)
        }
        Expression::InList { expr, list, .. } => {
            check_reference(expr, scope, site)?;
            for item in list {
                check_reference(item, scope, site)?;
            }
            Ok(())
        }
        Expression::Case { branches, else_branch, .. } => {
            for (when, then) in branches {
                check_reference(when, scope, site)?;
                check_reference(then, scope, site)?;
            }
            if let Some(els) = else_branch {
                check_reference(els, scope, site)?;
            }
            Ok(())
        }
        Expression::Literal(_)
        | Expression::Wildcard { .. }
        | Expression::Subquery { .. }
        | Expression::InSubquery { .. }
        | Expression::Exists { .. }
        | Expression::Error { .. } => Ok(()),
    }
}

fn check_identifier(identifier: &Identifier, scope: &CteScope, site: RefSite) -> Result<()> {
    let name = identifier.name.as_ref();
    if name.starts_with(PARAM_PREFIX) {
        return Ok(());
    }
    let Some(local) = scope.frames.last()
    else {
        return Ok(());
    };
    let sources = local.references.join(", ");
    let Some((qualifier, _)) = name.split_once('.')
    else {
        return if local.require_qualification {
            Err(sql_error(format!(
                "column reference `{name}` must be qualified with a row-source alias (available sources: {sources})"
            )))
        }
        else {
            Ok(())
        };
    };
    if local.references.iter().any(|reference| reference == qualifier) {
        return Ok(());
    }
    // Innermost first, skipping the statement being checked.
    let outer = scope.frames[..scope.frames.len() - 1]
        .iter()
        .enumerate()
        .rev()
        .find(|(_, frame)| frame.references.iter().any(|reference| reference == qualifier));
    let Some((index, frame)) = outer
    else {
        return Err(sql_error(format!(
            "`{name}` names `{qualifier}`, which is not a row source of this statement (available sources: {sources})"
        )));
    };
    // Per-row re-execution binds a subquery to the row of the statement that
    // directly encloses it, which must be the outermost statement.
    if index != 0 || scope.frames.len() != 2 {
        return Err(sql_error(format!(
            "`{name}` reads a column of a statement two or more levels out: YYDB re-runs a correlated subquery once per row of its enclosing statement and supports a single level only"
        )));
    }
    if let RefSite::WholeResult(clause) = site {
        return Err(sql_error(format!(
            "`{name}` reads a column of the enclosing statement inside `{clause}`, which decides the whole subquery result instead of one row: a correlated subquery may only read outer columns in its row-level expressions"
        )));
    }
    if frame.aggregate {
        return Err(sql_error(format!(
            "`{name}` reads a column of the enclosing statement, which collapses its rows (`GROUP BY` or an aggregate function), so no single row can be handed to each subquery run"
        )));
    }
    local.reads_outer.set(true);
    Ok(())
}
