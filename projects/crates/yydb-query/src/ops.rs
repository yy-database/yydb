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
    /// Conjunction.
    And(Box<Pred>, Box<Pred>),
    /// Disjunction.
    Or(Box<Pred>, Box<Pred>),
    /// Logical negation, used by frontends that expose `NOT`.
    Not(Box<Pred>),
}

/// One projected value expression.
#[derive(Debug, Clone, PartialEq, Eq)]
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
}

/// One projected output field.
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// One `SET column = value` pair of an update operator.
#[derive(Debug, Clone, PartialEq)]
pub struct WriteAssignment {
    /// Column name being replaced.
    pub field: String,
    /// New value (literal or parameter slot).
    pub value: BoundValue,
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
