//! Phase 1 read query physical ops (backend-neutral, non-SQL).

use std::collections::BTreeMap;

use yydb_types::Value;

/// One logical query result row.
pub type QueryRow = BTreeMap<String, Value>;

/// Comparison operator for filter predicates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// Bound value: either a literal baked into the plan at prepare time, or a
/// named/slot parameter resolved at bind time.
#[derive(Debug, Clone, PartialEq)]
pub enum BoundValue {
    Literal(Value),
    Slot {
        /// Positional index into the bound parameter slice.
        index: u32,
        /// Declared type from the parameter descriptor. Used at exec time to
        /// assert the caller-supplied value matches.
        expected_ty: SlotTy,
    },
}

/// Declared type for a parameter slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotTy {
    Bool,
    I64,
    Text,
    Null,
}

/// Convert a `SlotTy` to the display string used in parameter errors.
impl SlotTy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::I64 => "i64",
            Self::Text => "text",
            Self::Null => "null",
        }
    }

    /// Returns true iff `value.ty()` matches this slot type.
    pub fn matches(self, value: &Value) -> bool {
        match (self, value) {
            (Self::Bool, Value::Bool(_)) => true,
            (Self::I64, Value::I64(_)) => true,
            (Self::Text, Value::Text(_)) => true,
            (Self::Null, Value::Null) => true,
            _ => false,
        }
    }
}

/// Backend-neutral filter predicate (Phase 1 subset).
#[derive(Debug, Clone, PartialEq)]
pub enum Pred {
    True,
    False,
    FieldBool {
        path: Vec<String>,
        value: BoundValue,
    },
    FieldCmp {
        path: Vec<String>,
        op: CmpOp,
        value: BoundValue,
    },
    And(Box<Pred>, Box<Pred>),
    Or(Box<Pred>, Box<Pred>),
}

/// One projected value expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectExpr {
    /// Read a scalar field path from the current row context.
    Scalar { path: Vec<String> },
    /// Project nested fields from a referenced row at `path`.
    Nested {
        path: Vec<String>,
        fields: Vec<ProjectField>,
    },
}

/// One projected output field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectField {
    pub name: String,
    pub expr: ProjectExpr,
}

/// Sort key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortKey {
    pub field: String,
    pub ascending: bool,
}

/// Closed physical op set for Phase 1 read pipelines.
#[derive(Debug, Clone, PartialEq)]
pub enum QueryOp {
    Scan { table: String },
    Filter { predicate: Pred },
    Project { fields: Vec<ProjectField> },
    Sort { keys: Vec<SortKey> },
    Skip { count: BoundValue },
    Take { count: BoundValue },
    Collect,
}
