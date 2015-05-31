//! Backend-neutral physical operators shared by every YYDB query frontend.
//!
//! Frontends (VOS and SQL) only *lower* into these operators. Nothing in this
//! module knows about either authoring language, and no operator may carry a
//! frontend-specific construct that execution would have to interpret.

use std::collections::BTreeMap;

use yydb_types::Value;

/// One logical query result row.
pub type QueryRow = BTreeMap<String, Value>;

/// Leading component a correlated column path carries so execution reads it from
/// the enclosing statement's row instead of the row under test.
pub const OUTER_ROW_MARKER: &str = "__yydb_outer__";

/// Comparison operator for filter predicates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    /// `left = right`.
    Eq,
    /// `left != right`.
    Ne,
    /// `left < right`.
    Lt,
    /// `left <= right`.
    Le,
    /// `left > right`.
    Gt,
    /// `left >= right`.
    Ge,
}

/// Bound value: either a literal baked into the plan at prepare time, or a
/// named/slot parameter resolved at bind time.
#[derive(Debug, Clone, PartialEq)]
pub enum BoundValue {
    /// Compile-time constant.
    Literal(Value),
    /// Placeholder resolved from the bound parameter slice.
    Slot {
        /// Positional index into the bound parameter slice.
        index: u32,
        /// Declared type from the parameter descriptor. Used at exec time to
        /// assert the caller-supplied value matches.
        expected_ty: SlotTy,
    },
}

/// Declared type for a parameter slot. `Any` means the caller-supplied value
/// is accepted as-is at bind time with no compile-time type constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SlotTy {
    /// No type constraint.
    Any,
    /// Boolean.
    Bool,
    /// Signed integer.
    I64,
    /// UTF-8 text.
    Text,
    /// SQL `NULL`.
    Null,
}

/// Count expression for `LIMIT` / `OFFSET` positions: either a bake-in literal
/// or bind-time parameter, or an arbitrary scalar expression that is evaluated
/// at execution time (used for `LIMIT (SELECT …)` positions).
#[derive(Debug, Clone, PartialEq)]
pub enum CountExpr {
    /// Compile-time constant or bind-time parameter slot.
    Bound(BoundValue),
    /// Arbitrary scalar expression evaluated at execution time. Used for
    /// `LIMIT (SELECT COUNT(*) FROM …)` positions where the value is not
    /// known until the pipeline runs.
    Expr(Box<Expr>),
}

impl SlotTy {
    /// Stable lower-case name used in diagnostics and wire payloads.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Bool => "bool",
            Self::I64 => "i64",
            Self::Text => "text",
            Self::Null => "null",
        }
    }

    /// Returns true iff `value.ty()` matches this slot type. `Any` accepts everything.
    pub fn matches(self, value: &Value) -> bool {
        match self {
            Self::Any => true,
            Self::Bool => matches!(value, Value::Bool(_)),
            Self::I64 => matches!(value, Value::I64(_) | Value::U64(_)),
            Self::Text => matches!(value, Value::Text(_)),
            Self::Null => matches!(value, Value::Null),
        }
    }
}

/// Backend-neutral filter predicate.
#[derive(Debug, Clone, PartialEq)]
pub enum Pred {
    /// Always true.
    True,
    /// Always false.
    False,
    /// Truth test on a boolean column: `column IS TRUE`.
    FieldBool {
        /// Field path relative to the row being filtered.
        path: Vec<String>,
        /// Expected boolean value.
        value: BoundValue,
    },
    /// Comparison between a row field and a value.
    FieldCmp {
        /// Field path relative to the row being filtered.
        path: Vec<String>,
        /// Comparison operator.
        op: CmpOp,
        /// Right-hand side.
        value: BoundValue,
    },
    /// Comparison between two row fields (used by JOIN ON conditions).
    FieldCmpField {
        /// Left field path.
        left_path: Vec<String>,
        /// Comparison operator.
        op: CmpOp,
        /// Right field path.
        right_path: Vec<String>,
    },
    /// Conjunction.
    And(Box<Pred>, Box<Pred>),
    /// Disjunction.
    Or(Box<Pred>, Box<Pred>),
    /// Logical negation, used by frontends that expose `NOT`.
    Not(Box<Pred>),
    /// `[NOT] EXISTS (subquery)`.
    ///
    /// The subquery keeps its own pipeline. An uncorrelated one is resolved once
    /// before any row is tested. A `correlated` one reads columns of the row
    /// under test, so execution re-runs its pipeline for every row and hands
    /// that row in as the enclosing row.
    Exists {
        /// Lowered subquery pipeline.
        ops: Vec<QueryOp>,
        /// Whether the test is negated (`NOT EXISTS`).
        negated: bool,
        /// Whether the subquery reads columns of the row under test.
        correlated: bool,
    },
    /// `expr [NOT] IN (subquery)`.
    ///
    /// Like [`Pred::Exists`] an uncorrelated subquery is resolved once, before
    /// any row is tested, into the set of values it produces. A `correlated` one
    /// produces that set once per row, from the row under test.
    InSubquery {
        /// Field path tested against the subquery output.
        path: Vec<String>,
        /// Lowered subquery pipeline, ending in [`QueryOp::Collect`].
        ops: Vec<QueryOp>,
        /// Whether the test is negated (`NOT IN`).
        negated: bool,
        /// Whether the subquery reads columns of the row under test.
        correlated: bool,
    },
    /// `expr [NOT] IN (<materialized values>)`, produced by resolving
    /// [`Pred::InSubquery`]. Never appears in a freshly lowered plan.
    ///
    /// The test follows SQL three-valued logic: it is unknown when the tested
    /// value is `NULL` or when no value matches while the set contains `NULL`,
    /// and it is always decided for an empty set. A `WHERE` clause keeps a row
    /// only on `TRUE`, so an unknown test drops it.
    InSet {
        /// Field path tested against the materialized values.
        path: Vec<String>,
        /// Values the subquery produced, in row order.
        values: Vec<Value>,
        /// Whether the test is negated (`NOT IN`).
        negated: bool,
    },
    /// `field <op> (subquery)`.
    ///
    /// Like [`Pred::Exists`] an uncorrelated subquery is resolved once before
    /// any row is tested. A `correlated` one produces its value once per row,
    /// from the row under test. Either way it must project exactly one column
    /// and produce at most one row: zero rows yield `NULL`, more than one row is
    /// an error rather than a silent first-row pick.
    FieldCmpSubquery {
        /// Field path on the left-hand side.
        path: Vec<String>,
        /// Comparison operator.
        op: CmpOp,
        /// Lowered subquery pipeline, ending in [`QueryOp::Collect`].
        ops: Vec<QueryOp>,
        /// Whether the subquery reads columns of the row under test.
        correlated: bool,
    },
    /// `field <op> <scalar>`, produced by resolving
    /// [`Pred::FieldCmpSubquery`]. Never appears in a freshly lowered plan.
    ///
    /// Unlike [`Pred::FieldCmp`] this follows SQL three-valued logic: a `NULL`
    /// on either side makes the comparison unknown, so a `WHERE` clause drops
    /// the row. `FieldCmp` cannot express that because `IS NULL` lowers onto it.
    FieldCmpScalar {
        /// Field path on the left-hand side.
        path: Vec<String>,
        /// Comparison operator.
        op: CmpOp,
        /// Value the subquery produced.
        value: Value,
    },
    /// Arbitrary scalar expression compared against a bound value.
    ///
    /// Used by `WHERE LOWER(email) = 'a'` or `WHERE score + 1 >= 10`. Like
    /// [`Pred::FieldCmpScalar`] this follows SQL three-valued logic: a `NULL`
    /// on either side makes the comparison unknown, so a `WHERE` clause drops
    /// the row.
    ExprCmp {
        /// Expression evaluated row by row.
        expr: Box<Expr>,
        /// Comparison operator.
        op: CmpOp,
        /// Right-hand side value (literal or bind-time parameter slot).
        value: BoundValue,
    },
    /// Arbitrary scalar expression tested for `[NOT] NULL`.
    ///
    /// Unlike [`Pred::FieldCmp`] with `Value::Null`, `IS NULL` is its own
    /// three-valued operator: `expr IS NULL` is `TRUE` on `NULL`, `FALSE` on
    /// non-`NULL`, never unknown. `IS NOT NULL` mirrors that.
    ExprIsNull {
        /// Expression evaluated row by row.
        expr: Box<Expr>,
        /// Whether the test is negated (`IS NOT NULL`).
        negated: bool,
    },
    /// Arbitrary scalar expression tested with `[NOT] BETWEEN low AND high`.
    ///
    /// The comparison follows SQL three-valued logic — a `NULL` in `expr`,
    /// `low` or `high` makes the result unknown, so a `WHERE` clause drops
    /// the row.
    ExprBetween {
        /// Expression evaluated row by row.
        expr: Box<Expr>,
        /// Lower bound (literal or bind-time parameter slot).
        low: BoundValue,
        /// Upper bound.
        high: BoundValue,
        /// Whether the test is negated (`NOT BETWEEN`).
        negated: bool,
    },
    /// Arbitrary scalar expression tested with `[NOT] IN (<values>)`.
    ExprInList {
        /// Expression evaluated row by row.
        expr: Box<Expr>,
        /// Literal / bind-time values the expression may match.
        list: Vec<BoundValue>,
        /// Whether the test is negated (`NOT IN`).
        negated: bool,
    },
    /// Arbitrary scalar expression tested with `[NOT] IN (subquery)`.
    ExprInSubquery {
        /// Expression evaluated row by row.
        expr: Box<Expr>,
        /// Lowered subquery pipeline, ending in [`QueryOp::Collect`].
        ops: Vec<QueryOp>,
        /// Whether the test is negated (`NOT IN`).
        negated: bool,
        /// Whether the subquery reads columns of the row under test.
        correlated: bool,
    },
    /// Arbitrary scalar expression tested against materialized IN values.
    ///
    /// Produced by resolving [`Pred::ExprInSubquery`]. Never appears in a
    /// freshly lowered plan. Follows the same three-valued semantics as
    /// [`Pred::InSet`].
    ExprInSet {
        /// Expression evaluated row by row.
        expr: Box<Expr>,
        /// Values the subquery produced, in row order.
        values: Vec<Value>,
        /// Whether the test is negated (`NOT IN`).
        negated: bool,
    },
    /// `WHERE (SELECT …)` — a scalar subquery evaluated as a boolean predicate.
    ///
    /// The subquery must project exactly one column. Zero rows yields `NULL`
    /// (unknown → row dropped); one row yields `TRUE` when the value is truthy,
    /// `FALSE` otherwise. More than one row is an error. An uncorrelated
    /// subquery is resolved once before any row is tested; a `correlated` one
    /// produces its value once per row.
    SubqueryAsBool {
        /// Lowered subquery pipeline, ending in [`QueryOp::Collect`].
        ops: Vec<QueryOp>,
        /// Whether the subquery reads columns of the row under test.
        correlated: bool,
    },
}

/// One projected value expression.
#[derive(Debug, Clone, PartialEq)]
pub enum ProjectExpr {
    /// Read a scalar field path from the current row context.
    Scalar {
        /// Field path relative to the current row.
        path: Vec<String>,
    },
    /// Project nested fields from a referenced row at `path`.
    Nested {
        /// Field path to the referenced row.
        path: Vec<String>,
        /// Fields projected from that row.
        fields: Vec<ProjectField>,
    },
    /// Compute an arbitrary scalar expression over the current row.
    Computed(Box<Expr>),
}

/// Arbitrary scalar expression that can appear inside SELECT projection
/// (`CASE WHEN ... THEN ... ELSE ... END`, `CAST(expr AS type)`, and the
/// primitive terminals they recurse into). Unlike [`BoundValue`] this can
/// reference columns so the executor resolves against each row.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Read a column from the current row.
    Column {
        /// Field path (may be dotted for qualified names).
        path: Vec<String>,
    },
    /// Plan-time literal or bind-time parameter slot.
    Bound(BoundValue),
    /// `CASE WHEN ... THEN ... [ELSE ...] END`.
    Case {
        /// WHEN/THEN pairs evaluated left to right. First `when` that is truthy
        /// yields the corresponding `then`.
        branches: Vec<(Box<Expr>, Box<Expr>)>,
        /// Optional ELSE fallback. Evaluates to `NULL` when absent and every
        /// branch's WHEN is falsy.
        else_branch: Option<Box<Expr>>,
    },
    /// `CAST(expr AS <target_type>)`.
    Cast {
        /// Expression to convert.
        expr: Box<Expr>,
        /// Target type name, normalised to upper-case by the SQL lowering
        /// (`INTEGER`, `REAL`, `TEXT`, `BLOB`).
        target: String,
    },
    /// Binary operation used by CASE WHEN predicates (`score > 20`,
    /// `active AND score > 10`, …) and by arithmetic projections
    /// (`SELECT score + 1`). Comparison, logical and arithmetic operators all
    /// live here so expressions nest arbitrarily.
    Binary {
        /// Operator kind — arithmetic, comparison or logical.
        op: BinaryOp,
        /// Left operand (may itself be a Compound).
        left: Box<Expr>,
        /// Right operand (may itself be a Compound).
        right: Box<Expr>,
    },
    /// Logical NOT — wraps a predicate expression.
    Not(Box<Expr>),
    /// Scalar function call over the current row (`LOWER(email)`).
    Function {
        /// Function to apply.
        function: ScalarFunction,
        /// Argument expressions, in call order. Arity is checked at lowering.
        args: Vec<Expr>,
    },
    /// Scalar subquery `(SELECT ...)` used as a value.
    ///
    /// An uncorrelated subquery is resolved once per projection rather than once
    /// per row. A `correlated` one produces its value once per row, from the row
    /// being projected. Either way it must project exactly one column and
    /// produce at most one row: zero rows yield `NULL`, more than one row is an
    /// error rather than a silent first-row pick.
    Subquery {
        /// Lowered subquery pipeline, ending in [`QueryOp::Collect`].
        ops: Vec<QueryOp>,
        /// Whether the subquery reads columns of the row being projected.
        correlated: bool,
    },
}

/// Operator for [`Expr::Binary`]. Covers the arithmetic forms a projection may
/// compute, the comparison forms CASE WHEN predicates need, and the logical
/// connectives that join those predicates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    /// Addition (`+`).
    Add,
    /// Subtraction (`-`).
    Sub,
    /// Multiplication (`*`).
    Mul,
    /// Division (`/`).
    Div,
    /// Remainder (`%`).
    Rem,
    /// Equality (`=`, `==`).
    Eq,
    /// Inequality (`<>`, `!=`).
    Ne,
    /// Less than (`<`).
    Lt,
    /// Less than or equal (`<=`).
    Le,
    /// Greater than (`>`).
    Gt,
    /// Greater than or equal (`>=`).
    Ge,
    /// Logical AND.
    And,
    /// Logical OR.
    Or,
}

/// Scalar functions the query executor evaluates row by row.
///
/// Every function propagates `NULL`, so a `NULL` argument makes the whole result
/// `NULL`. [`ScalarFunction::Coalesce`] is the exception: it evaluates its
/// arguments left to right and returns the first non-`NULL` one, so `NULL` does
/// not by itself decide the result. Argument kinds are checked by the executor
/// because lowering has no column types: the string functions accept text only
/// (and integer positions for [`ScalarFunction::Substr`]), while the date part
/// extractors accept `date` and `datetime` only. Any other kind is an error
/// rather than an implicit conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarFunction {
    /// `LOWER(text)`: lower-case the whole string.
    Lower,
    /// `UPPER(text)`: upper-case the whole string.
    Upper,
    /// `LENGTH(text)`: number of Unicode characters, as [`Value::I64`].
    Length,
    /// `CHAR_LENGTH(text)`: number of Unicode characters, as [`Value::I64`].
    /// Counts exactly like [`ScalarFunction::Length`].
    CharLength,
    /// `SUBSTR(text, start[, length])`: 1-based character slice, where a
    /// negative `start` counts from the end of the string.
    Substr,
    /// `TRIM(text)`: strip leading and trailing whitespace.
    Trim,
    /// `REPLACE(text, from, to)`: replace every occurrence of `from` with `to`.
    /// An empty `from` leaves `text` untouched.
    Replace,
    /// `COALESCE(a, b, …)`: the first non-`NULL` argument, or `NULL` when every
    /// argument is `NULL`.
    Coalesce,
    /// `CONCAT(a, b, …)`: concatenate two or more strings.
    Concat,
    /// `YEAR(date_or_datetime)`: calendar year, as [`Value::I64`].
    Year,
    /// `MONTH(date_or_datetime)`: calendar month (`1`–`12`), as [`Value::I64`].
    Month,
    /// `DAY(date_or_datetime)`: day of the month (`1`–`31`), as [`Value::I64`].
    Day,
    /// `NOW()`: frozen UTC timestamp captured at query start, as [`Value::Timestamp`].
    Now,
    /// `CURRENT_TIMESTAMP`: alias for [`ScalarFunction::Now`].
    CurrentTimestamp,
    /// `CURRENT_DATE`: frozen UTC date part of the query start timestamp, as [`Value::Date`].
    CurrentDate,
    /// `CURRENT_TIME`: frozen UTC wall-clock time part of the query start timestamp, as [`Value::Time`].
    CurrentTime,
}

/// Set operator combining two row pipelines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetOpKind {
    /// `UNION`: every row of both operands, duplicates removed.
    Union,
    /// `UNION ALL`: every row of both operands, duplicates kept.
    UnionAll,
    /// `INTERSECT`: rows present in both operands, duplicates removed.
    Intersect,
    /// `EXCEPT`: rows of the left operand that the right operand lacks,
    /// duplicates removed.
    Except,
}

impl SetOpKind {
    /// Stable upper-case spelling used in diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Union => "UNION",
            Self::UnionAll => "UNION ALL",
            Self::Intersect => "INTERSECT",
            Self::Except => "EXCEPT",
        }
    }
}

/// One projected output field.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectField {
    /// Output column name.
    pub name: String,
    /// Value expression producing the column.
    pub expr: ProjectExpr,
}

/// Sort key.
#[derive(Debug, Clone, PartialEq)]
pub struct SortKey {
    /// Output field name to order by when `expr` is absent. Also used as a
    /// diagnostic hint and as the path to compare when the row already carries
    /// the computed column (ORDER BY alias).
    pub field: String,
    /// When present the sort value is computed from this expression per row
    /// instead of being read from the row under `field`.
    pub expr: Option<Expr>,
    /// Whether the order is ascending.
    pub ascending: bool,
}

/// Kind of SQL join.
///
/// The joined row keeps the base (left) table columns bare and prefixes the
/// joined (right) table columns with that table's name, so padding never
/// renames anything: an unmatched row from either side simply carries `NULL`
/// for the other side's columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinKind {
    /// `INNER JOIN` — rows with no match on either side are dropped.
    Inner,
    /// `LEFT JOIN` — every left-hand row is kept; right-hand columns become
    /// `NULL` when there is no matching row.
    Left,
    /// `RIGHT JOIN` — every right-hand row is kept; left-hand columns become
    /// `NULL` when there is no matching row.
    Right,
    /// `FULL JOIN` — unmatched rows from both sides are kept, with `NULL` for
    /// the columns of the side that has no matching row.
    Full,
}

/// One aggregate function call.
#[derive(Debug, Clone, PartialEq)]
pub enum AggregateField {
    /// `COUNT(*)` or `COUNT(expr)`. `expr` is `None` for `COUNT(*)`.
    Count {
        /// Output column name (from SELECT alias, or `count` if none).
        name: String,
        /// Optional expression to count non-null values of.
        expr: Option<ProjectExpr>,
    },
    /// `COUNT(DISTINCT expr)` — distinct non-null values of the expression.
    /// `SELECT * DISTINCT COUNT(DISTINCT *)` is semantically unclear and is
    /// rejected at lowering time (expressions are required).
    CountDistinct {
        /// Output column name (from SELECT alias, or `count_distinct` if none).
        name: String,
        /// Expression whose distinct non-null values are counted.
        expr: ProjectExpr,
    },
    /// `SUM(expr)` — sums non-null numeric values.
    Sum {
        /// Output column name (from SELECT alias, or `sum` if none).
        name: String,
        /// Expression to aggregate.
        expr: ProjectExpr,
    },
    /// `AVG(expr)` — average of non-null numeric values. Returns `NULL` when
    /// the group has no non-null values. Output is a `Value::F64`.
    Avg {
        /// Output column name (from SELECT alias, or `avg` if none).
        name: String,
        /// Expression to aggregate.
        expr: ProjectExpr,
    },
    /// `MIN(expr)` — smallest non-null value. Keeps the picked value's type and
    /// returns `NULL` when the group holds no value.
    Min {
        /// Output column name (from SELECT alias, or `min` if none).
        name: String,
        /// Expression to aggregate.
        expr: ProjectExpr,
    },
    /// `MAX(expr)` — largest non-null value. Keeps the picked value's type and
    /// returns `NULL` when the group holds no value.
    Max {
        /// Output column name (from SELECT alias, or `max` if none).
        name: String,
        /// Expression to aggregate.
        expr: ProjectExpr,
    },
}

impl AggregateField {
    /// Output column name this aggregate produces.
    pub fn name(&self) -> &str {
        match self {
            Self::Count { name, .. }
            | Self::CountDistinct { name, .. }
            | Self::Sum { name, .. }
            | Self::Avg { name, .. }
            | Self::Min { name, .. }
            | Self::Max { name, .. } => name,
        }
    }
}

/// Window function one [`WindowField`] computes.
///
/// The ranking functions ignore the frame; the aggregates evaluate it, so they
/// follow the same type rules as the plain [`AggregateField`] path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowFunction {
    /// `ROW_NUMBER()`: 1-based position inside the ordered partition.
    RowNumber,
    /// `RANK()`: 1-based position where peers share a rank and leave a gap.
    Rank,
    /// `DENSE_RANK()`: 1-based peer group number with no gaps.
    DenseRank,
    /// `SUM(expr)` over the frame.
    Sum,
    /// `COUNT(*)` or `COUNT(expr)` over the frame.
    Count,
    /// `AVG(expr)` over the frame. Always a [`Value::F64`] outside the empty case.
    Avg,
    /// `MIN(expr)` over the frame.
    Min,
    /// `MAX(expr)` over the frame.
    Max,
}

impl WindowFunction {
    /// Stable upper-case spelling used in diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RowNumber => "ROW_NUMBER",
            Self::Rank => "RANK",
            Self::DenseRank => "DENSE_RANK",
            Self::Sum => "SUM",
            Self::Count => "COUNT",
            Self::Avg => "AVG",
            Self::Min => "MIN",
            Self::Max => "MAX",
        }
    }

    /// Whether the function reads a value expression from each row.
    ///
    /// The ranking functions do not, and neither does `COUNT(*)`.
    pub fn reads_argument(self) -> bool {
        !matches!(self, Self::RowNumber | Self::Rank | Self::DenseRank)
    }
}

/// One `ORDER BY` key of a window specification.
///
/// The comparison and the `NULL` placement are the shared ones used by
/// [`QueryOp::Sort`], so a window never orders values by its own rule.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowOrderKey {
    /// Value expression read from the current row.
    pub expr: ProjectExpr,
    /// Whether the order is ascending.
    pub ascending: bool,
}

/// One bound of an explicit `ROWS` frame.
///
/// Offsets count rows, which is why `RANGE` frames are rejected during lowering
/// instead of being read as a row offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowFrameBound {
    /// `UNBOUNDED PRECEDING`: the first row of the partition.
    UnboundedPreceding,
    /// `n PRECEDING`: `n` rows before the current row.
    Preceding(u64),
    /// `CURRENT ROW`: the current row.
    CurrentRow,
    /// `n FOLLOWING`: `n` rows after the current row.
    Following(u64),
    /// `UNBOUNDED FOLLOWING`: the last row of the partition.
    UnboundedFollowing,
}

/// Explicit `ROWS` frame of the current row.
///
/// Both bounds are inclusive. A frame that starts after its end is empty, which
/// the executor reports as the aggregate's empty-input result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowFrame {
    /// Bound the frame starts at.
    pub start: WindowFrameBound,
    /// Bound the frame ends at.
    pub end: WindowFrameBound,
}

/// Partitioning, ordering and frame one window function is evaluated over.
///
/// Without a `frame` the SQL default applies: the whole partition when there is
/// no `order_by`, otherwise everything up to and including the current row's
/// last peer.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowSpec {
    /// `PARTITION BY` value expressions; empty means one single partition.
    pub partition_by: Vec<ProjectExpr>,
    /// `ORDER BY` keys; empty means every partition row is a peer.
    pub order_by: Vec<WindowOrderKey>,
    /// Explicit `ROWS` frame, absent for the default frame.
    pub frame: Option<WindowFrame>,
}

/// One window computation over the current rows.
///
/// Execution appends the value as a new column named `name` and keeps the row
/// order unchanged, so a following [`QueryOp::Project`] renames columns only.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowField {
    /// Output column name (from SELECT alias, or the lower-case function name).
    pub name: String,
    /// Function to compute.
    pub function: WindowFunction,
    /// Value expression per row. `None` only for the ranking functions and
    /// `COUNT(*)`.
    pub argument: Option<ProjectExpr>,
    /// Window the function is evaluated over.
    pub spec: WindowSpec,
}

/// Closed physical op set for read pipelines.
#[derive(Debug, Clone, PartialEq)]
pub enum QueryOp {
    /// Load every row of `table`.
    Scan {
        /// Table name as written by the frontend.
        table: String,
    },
    /// Combine the rows of two nested pipelines with a set operator
    /// (`UNION [ALL]` / `INTERSECT` / `EXCEPT`).
    ///
    /// Both operands are materialized before they are combined, and the output
    /// keeps the left operand's column names: the combined rows are positionally
    /// aligned, so the two operands must produce the same number of columns of
    /// compatible types. A type disagreement is rejected rather than coerced.
    SetOperation {
        /// Set operator applied to both operands.
        kind: SetOpKind,
        /// Left-hand row pipeline, ending in [`QueryOp::Collect`].
        left: Vec<QueryOp>,
        /// Right-hand row pipeline, ending in [`QueryOp::Collect`].
        right: Vec<QueryOp>,
        /// Output column names of the left operand in projection order. `None`
        /// means the left operand projects `*`, whose columns are only known
        /// from the executed rows.
        columns: Option<Vec<String>>,
        /// Output column names of the right operand in its own projection
        /// order, used for the positional alignment. `None` means `*`.
        right_columns: Option<Vec<String>>,
    },
    /// Rename every column of the current rows to `<prefix>.<column>`.
    ///
    /// A row source that carries an alias contributes only alias-qualified
    /// columns, so two occurrences of one table remain distinguishable and the
    /// join keeps a single prefix rule for every side.
    Qualify {
        /// Prefix every column of the current rows is renamed with.
        prefix: String,
    },
    /// Drop rows that do not satisfy the predicate.
    Filter {
        /// Row predicate.
        predicate: Pred,
    },
    /// Merge the accumulating row stream with another table via a join.
    Join {
        /// Join kind.
        kind: JoinKind,
        /// Right-hand table name as written by the frontend.
        right: String,
        /// Optional alias for the right-hand table (used as column prefix
        /// in the joined rows when supplied).
        right_alias: Option<String>,
        /// Join condition evaluated against each left/right row pair.
        on: Pred,
    },
    /// Merge the accumulating row stream with the rows a nested pipeline
    /// produces (`JOIN (SELECT ...) x ON ...`).
    ///
    /// The nested pipeline is materialized before the first row pair is
    /// matched, so a derived table never reads the left side of its own join.
    JoinSubquery {
        /// Join kind.
        kind: JoinKind,
        /// Right-hand row pipeline, ending in [`QueryOp::Collect`].
        right: Vec<QueryOp>,
        /// Reference the right-hand columns are prefixed with.
        right_alias: String,
        /// Join condition evaluated against each left/right row pair.
        on: Pred,
    },
    /// Group rows by zero or more keys and compute aggregate values.
    Aggregate {
        /// GROUP BY columns (empty means a single aggregate over all rows).
        group_keys: Vec<ProjectField>,
        /// Aggregate function outputs, in SELECT order.
        aggregates: Vec<AggregateField>,
    },
    /// Compute window values over the current rows.
    ///
    /// The rows keep their order and gain one column per field, so the operator
    /// never reorders a result. Each field is evaluated over its own partition
    /// and frame, and the ranking functions ignore the frame entirely.
    Window {
        /// Window computations, in SELECT order.
        fields: Vec<WindowField>,
    },
    /// Replace each row with the projected column set.
    Project {
        /// Columns to emit.
        fields: Vec<ProjectField>,
    },
    /// Order rows by the given keys.
    Sort {
        /// Sort keys in priority order.
        keys: Vec<SortKey>,
    },
    /// Drop the first `count` rows.
    Skip {
        /// Number of rows to drop — either a bake-in literal/parameter slot,
        /// or an expression evaluated at execution time (for `LIMIT (SELECT …)`).
        count: CountExpr,
    },
    /// Keep only the first `count` rows.
    Take {
        /// Number of rows to keep — either a bake-in literal/parameter slot,
        /// or an expression evaluated at execution time (for `LIMIT (SELECT …)`).
        count: CountExpr,
    },
    /// Materialize the pipeline result.
    Collect,
}

/// One column value supplied by a write operator.
#[derive(Debug, Clone, PartialEq)]
pub struct WriteField {
    /// Column name as written by the frontend; resolved against the catalog at
    /// execution time.
    pub name: String,
    /// Literal or parameter slot to store.
    pub value: BoundValue,
}

/// Arithmetic operators available inside `UPDATE … SET col = …` value expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteBinOp {
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
}

/// Expression producing a single scalar value used as a write target.
///
/// Unlike [`BoundValue`] this can reference columns of the row being updated,
/// which is how `UPDATE t SET score = score + 1` survives lowering into a
/// backend-neutral plan. Literals and parameter slots still flow through
/// [`BoundValue`] — [`WriteExpr::Bound`] wraps it so the arithmetic cases can
/// reuse the existing bound-value resolution at execution time.
#[derive(Debug, Clone, PartialEq)]
pub enum WriteExpr {
    /// A plan-time literal or bind-time parameter.
    Bound(BoundValue),
    /// Read a column from the row being updated.
    Column { path: Vec<String> },
    /// Apply an arithmetic operator to two sub-expressions.
    Binary { op: WriteBinOp, left: Box<WriteExpr>, right: Box<WriteExpr> },
}

/// One `SET column = value` pair of an update operator.
#[derive(Debug, Clone, PartialEq)]
pub struct WriteAssignment {
    /// Column name being replaced.
    pub field: String,
    /// New value expression — may read from the row being updated.
    pub value: WriteExpr,
}

/// Action an `INSERT … ON CONFLICT` clause applies to a conflicting row.
#[derive(Debug, Clone, PartialEq)]
pub enum ConflictAction {
    /// `DO NOTHING`: the conflicting row is skipped and nothing is written.
    DoNothing,
    /// `DO UPDATE SET … [WHERE …]`: the conflicting stored row is updated with
    /// the assignments, and the predicate can drop the update.
    DoUpdate {
        /// Assignments applied to the conflicting row. They may read the
        /// conflicting row's own columns (`SET score = score + 1`).
        assignments: Vec<WriteAssignment>,
        /// Optional predicate over the conflicting row; when it evaluates
        /// false the row is left untouched and the incoming row is dropped.
        predicate: Option<Pred>,
    },
}

/// `INSERT … ON CONFLICT (columns) …` clause.
///
/// The columns are the conflict target as written by the frontend. Execution
/// rejects a target that no primary key or uniqueness rule covers, so a clause
/// never silently changes which conflict it reacts to.
#[derive(Debug, Clone, PartialEq)]
pub struct ConflictClause {
    /// Conflict target columns, in statement order.
    pub columns: Vec<String>,
    /// Action taken when an incoming row conflicts on the target.
    pub action: ConflictAction,
}

/// Rows an `INSERT … RETURNING` statement hands back.
#[derive(Debug, Clone, PartialEq)]
pub enum ReturningProjection {
    /// `RETURNING *`: every stored column of the written row.
    Star,
    /// `RETURNING <expression> [AS alias], …`, in statement order.
    Fields(Vec<ProjectField>),
}

/// Closed physical op set for write programs.
///
/// The `table` is the name written by the frontend; execution resolves it to a
/// catalog table and to that table's storage key. `predicate` is the same
/// backend-neutral filter used by read pipelines, so every frontend reuses one
/// predicate evaluator.
#[derive(Debug, Clone, PartialEq)]
pub enum WriteOp {
    /// Insert every row of one statement. Missing primary key is rejected at
    /// execution time, and the rows are published only when all of them pass.
    Insert {
        /// Target table as written by the frontend.
        table: String,
        /// Column values of each new row, one vector per `VALUES` row.
        rows: Vec<Vec<WriteField>>,
        /// Optional `ON CONFLICT` clause applied per row.
        conflict: Option<ConflictClause>,
        /// Optional `RETURNING` projection of the written rows.
        returning: Option<ReturningProjection>,
    },
    /// Insert the rows a read pipeline produces (`INSERT INTO … SELECT …`).
    ///
    /// The source pipeline is materialized before the first row is staged, so
    /// it reads the statement's starting snapshot even when it scans the target
    /// table. Its rows then follow the same staged publication as [`WriteOp::Insert`],
    /// which keeps the statement atomic.
    InsertFromQuery {
        /// Target table as written by the frontend.
        table: String,
        /// Explicit target columns in statement order. Empty means every
        /// catalog field of the target, in declaration order.
        columns: Vec<String>,
        /// Source row pipeline, ending in [`QueryOp::Collect`].
        source: Vec<QueryOp>,
        /// Output column names of `source` in SELECT order. `None` means the
        /// projection is `*`, whose columns are resolved from the scanned
        /// table at execution time.
        outputs: Option<Vec<String>>,
        /// Optional `ON CONFLICT` clause applied per produced row.
        conflict: Option<ConflictClause>,
        /// Optional `RETURNING` projection of the written rows.
        returning: Option<ReturningProjection>,
    },
    /// Replace the assigned columns of every row matching `predicate`
    /// (`None` means every row). Primary-key reassignment is rejected.
    Update {
        /// Target table as written by the frontend.
        table: String,
        /// Columns to replace.
        assignments: Vec<WriteAssignment>,
        /// Row filter; `None` selects every row.
        predicate: Option<Pred>,
    },
    /// Remove every row matching `predicate` (`None` means every row).
    Delete {
        /// Target table as written by the frontend.
        table: String,
        /// Row filter; `None` selects every row.
        predicate: Option<Pred>,
    },
}
