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

/// Literal kind for [`Pred::FieldCmp`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiteralKind {
    Bool,
    Int,
    Str,
    Null,
}

/// Backend-neutral filter predicate (Phase 1 subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pred {
    True,
    False,
    FieldBool {
        path: Vec<String>,
        value: bool,
    },
    FieldCmp {
        path: Vec<String>,
        op: CmpOp,
        literal: String,
        kind: LiteralKind,
    },
    And(Box<Pred>, Box<Pred>),
    Or(Box<Pred>, Box<Pred>),
}

/// One projected value expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectExpr {
    /// Read a scalar field path from the current row context.
    Scalar {
        path: Vec<String>,
    },
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryOp {
    Scan {
        table: String,
    },
    Filter {
        predicate: Pred,
    },
    Project {
        fields: Vec<ProjectField>,
    },
    Sort {
        keys: Vec<SortKey>,
    },
    Skip {
        count: u64,
    },
    Take {
        count: u64,
    },
    Collect,
}
