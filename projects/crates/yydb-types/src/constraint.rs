//! Catalog uniqueness metadata shared by schema catalog and write paths.

use std::{collections::BTreeSet, fmt};

/// A uniqueness rule over one or more columns of a single catalog type.
///
/// `columns` holds canonical catalog column names in declaration order.
/// `nulls_distinct` selects the SQL standard NULL rule:
///
/// - `true` (SQL standard): a row with any NULL in the key never conflicts.
/// - `false`: NULL is treated as an ordinary key component, so repeated NULLs
///   collide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UniqueConstraint {
    name: String,
    columns: Vec<String>,
    nulls_distinct: bool,
}

impl UniqueConstraint {
    /// Validates and builds a constraint.
    pub fn new(
        name: impl Into<String>,
        columns: Vec<String>,
        nulls_distinct: bool,
    ) -> Result<Self, UniqueConstraintError> {
        let name = name.into();
        if name.is_empty() {
            return Err(UniqueConstraintError::EmptyName);
        }
        if columns.is_empty() {
            return Err(UniqueConstraintError::EmptyColumns { name });
        }
        let mut seen = BTreeSet::new();
        for column in &columns {
            if column.is_empty() {
                return Err(UniqueConstraintError::EmptyColumn { name });
            }
            if !seen.insert(column.as_str()) {
                return Err(UniqueConstraintError::DuplicateColumn { name, column: column.clone() });
            }
        }
        Ok(Self { name, columns, nulls_distinct })
    }

    /// Constraint name exposed to hosts and diagnostics.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Ordered column set covered by this constraint.
    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    /// Whether NULL key components keep rows distinct (SQL standard) or collide.
    pub fn nulls_distinct(&self) -> bool {
        self.nulls_distinct
    }
}

impl fmt::Display for UniqueConstraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({}){}",
            self.name,
            self.columns.join(", "),
            if self.nulls_distinct { " nulls distinct" } else { " nulls equal" }
        )
    }
}

/// A [`UniqueConstraint`] bound to one catalog table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableUniqueConstraint {
    table: String,
    constraint: UniqueConstraint,
}

impl TableUniqueConstraint {
    /// Validates and binds a constraint to a storage table name.
    pub fn new(table: impl Into<String>, constraint: UniqueConstraint) -> Result<Self, UniqueConstraintError> {
        let table = table.into();
        if table.is_empty() {
            return Err(UniqueConstraintError::EmptyTable);
        }
        Ok(Self { table, constraint })
    }

    /// Storage table name.
    pub fn table(&self) -> &str {
        &self.table
    }

    /// Bound constraint.
    pub fn constraint(&self) -> &UniqueConstraint {
        &self.constraint
    }
}

/// Rejection raised when a [`UniqueConstraint`] declaration is malformed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum UniqueConstraintError {
    /// Constraint name is empty.
    EmptyName,
    /// Table name is empty.
    EmptyTable,
    /// Constraint covers no columns.
    EmptyColumns {
        /// Declared constraint name.
        name: String,
    },
    /// One column entry is empty.
    EmptyColumn {
        /// Declared constraint name.
        name: String,
    },
    /// The same column appears twice in one constraint.
    DuplicateColumn {
        /// Declared constraint name.
        name: String,
        /// Repeated column name.
        column: String,
    },
}

impl fmt::Display for UniqueConstraintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => f.write_str("unique constraint name must not be empty"),
            Self::EmptyTable => f.write_str("unique constraint table name must not be empty"),
            Self::EmptyColumns { name } => write!(f, "unique constraint `{name}` has no columns"),
            Self::EmptyColumn { name } => write!(f, "unique constraint `{name}` has an empty column"),
            Self::DuplicateColumn { name, column } => {
                write!(f, "unique constraint `{name}` repeats column `{column}`")
            }
        }
    }
}

impl std::error::Error for UniqueConstraintError {}

impl From<UniqueConstraintError> for crate::Error {
    fn from(error: UniqueConstraintError) -> Self {
        Self::Schema { message: error.to_string() }
    }
}
