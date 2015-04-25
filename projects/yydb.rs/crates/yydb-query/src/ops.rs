//! Backend-neutral physical operators shared by every YYDB query frontend.
//!
//! Frontends (VOS and SQL) only *lower* into these operators. Nothing in this
//! module knows about either authoring language, and no operator may carry a
//! frontend-specific construct that execution would have to interpret.

use std::collections::BTreeMap;

use yydb_types::Value;

/// One logical query result row.
pub type QueryRow = BTreeMap<String, Value>;

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
    /// Binary operation used for CASE WHEN predicates (`score > 20`,
    /// `active AND score > 10`, …). Both comparison and logical operators are
    /// here so WHEN branches can nest arbitrarily.
    Binary {
        /// Operator kind — comparison or logical.
        op: BinaryOp,
        /// Left operand (may itself be a Compound).
        left: Box<Expr>,
        /// Right operand (may itself be a Compound).
        right: Box<Expr>,
    },
    /// Logical NOT — wraps a predicate expression.
    Not(Box<Expr>),
}

/// Operator for [`Expr::Binary`]. Covers the comparison and logical forms
/// needed by CASE WHEN predicates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
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

/// One projected output field.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectField {
    /// Output column name.
    pub name: String,
    /// Value expression producing the column.
    pub expr: ProjectExpr,
}

/// Sort key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortKey {
    /// Output field name to order by.
    pub field: String,
    /// Whether the order is ascending.
    pub ascending: bool,
}

/// Kind of SQL join. Right / Full join still reject at lowering time; the
/// executor only implements [`JoinKind::Inner`] and [`JoinKind::Left`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinKind {
    /// `INNER JOIN` — rows with no match on either side are dropped.
    Inner,
    /// `LEFT JOIN` — every left-hand row is kept; right-hand columns become
    /// `NULL` when there is no matching row.
    Left,
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
}

/// Closed physical op set for read pipelines.
#[derive(Debug, Clone, PartialEq)]
pub enum QueryOp {
    /// Load every row of `table`.
    Scan {
        /// Table name as written by the frontend.
        table: String,
    },
    /// Drop rows that do not satisfy the predicate.
    Filter {
        /// Row predicate.
        predicate: Pred,
    },
    /// Merge the accumulating row stream with another table via a join.
    Join {
        /// Join kind — currently only inner is supported.
        kind: JoinKind,
        /// Right-hand table name as written by the frontend.
        right: String,
        /// Optional alias for the right-hand table (used as column prefix
        /// in the joined rows when supplied).
        right_alias: Option<String>,
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
        /// Number of rows to drop.
        count: BoundValue,
    },
    /// Keep only the first `count` rows.
    Take {
        /// Number of rows to keep.
        count: BoundValue,
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

/// Closed physical op set for write programs.
///
/// The `table` is the name written by the frontend; execution resolves it to a
/// catalog table and to that table's storage key. `predicate` is the same
/// backend-neutral filter used by read pipelines, so every frontend reuses one
/// predicate evaluator.
#[derive(Debug, Clone, PartialEq)]
pub enum WriteOp {
    /// Insert one row. Missing primary key is rejected at execution time.
    Insert {
        /// Target table as written by the frontend.
        table: String,
        /// Column values of the new row.
        fields: Vec<WriteField>,
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
