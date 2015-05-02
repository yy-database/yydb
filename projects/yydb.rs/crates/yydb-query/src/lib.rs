//! YYDB query layer: both frontends (VOS and SQL) lower into one shared
//! operator set, which executes against stored table rows.
//!
//! Sits above [`yydb-types`] and below the [`yydb`] facade. Storage hosts pass
//! their in-memory record map and persisted [`ResolvedContract`] into these entry points.
//!
//! | Stage | Module | Knows about |
//! |-------|--------|-------------|
//! | parse | `oak-vos` / `oak-sql` | authoring syntax |
//! | lower | `sql` / `lower` | frontend AST → `ops` |
//! | plan | `params` | parameters, kind, read/write set |
//! | execute | `exec` / `write` | rows and catalog only |

#![deny(missing_docs)]

mod catalog;
mod coerce;
mod constraints;
mod dml;
mod exec;
mod lower;
mod ops;
mod params;
mod resolve;
mod sql;
mod store;
mod write;

use std::collections::BTreeMap;

use vos::types::ResolvedContract;
use yydb_types::{TableUniqueConstraint, Value};

pub use catalog::resolved_contract_from_document;
pub use ops::{
    AggregateField, BoundValue, CmpOp, ConflictAction, ConflictClause, JoinKind, Pred, ProjectExpr, ProjectField,
    QueryOp, QueryRow, SlotTy, SortKey, WriteAssignment, WriteField, WriteOp,
};
pub use params::{
    BoundStatement, ParameterDescriptor, PreparedStatement, QueryKind, bind_named, bind_ordered, execute_prepared,
    execute_write_prepared, fingerprint, prepare,
};
#[doc(hidden)]
pub use params::{Dialect, prepare_sql};
#[doc(hidden)]
pub use sql::SqlPlan;
pub use store::upsert_row;
pub use write::WriteOutcome;

/// Execute a VOS read pipeline against stored table rows.
pub fn execute(
    source: &str,
    contract: Option<&ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
) -> yydb_types::Result<Vec<QueryRow>> {
    exec::execute(source, contract, records)
}

/// Execute unit-valued VOS write programs (for example `User { … }.insert()`).
pub fn execute_write(
    source: &str,
    contract: &ResolvedContract,
    schema_document: Option<&str>,
    records: &mut BTreeMap<String, Vec<u8>>,
) -> yydb_types::Result<()> {
    dml::execute(source, contract, schema_document, records)
}

/// Execute writes with host-declared uniqueness rules merged into document rules.
pub fn execute_write_with_constraints(
    source: &str,
    contract: &ResolvedContract,
    declared: &[yydb_types::TableUniqueConstraint],
    schema_document: Option<&str>,
    records: &mut BTreeMap<String, Vec<u8>>,
) -> yydb_types::Result<()> {
    dml::execute_with_constraints(source, contract, declared, schema_document, records)
}

/// Run a single `Type::insert({ … })` program and return the stored row when recognized.
pub fn try_insert_returning(
    source: &str,
    contract: &ResolvedContract,
    records: &mut BTreeMap<String, Vec<u8>>,
) -> yydb_types::Result<Option<Vec<QueryRow>>> {
    dml::try_insert_returning(source, contract, records)
}

/// Result of one SQL statement.
#[derive(Debug, Clone, Default, PartialEq)]
#[doc(hidden)]
pub struct SqlOutcome {
    /// Rows produced by a read statement (empty for writes).
    pub rows: Vec<QueryRow>,
    /// Rows affected by a write statement (zero for reads).
    pub affected: usize,
}

/// Execute one SQL statement against stored table rows.
///
/// `values` binds the statement's parameters in declaration order, so
/// `?` placeholders map to the first, second, … supplied value. Reads return
/// rows, writes return the affected-row count.
///
/// **Not part of the `@yydb/yydb` public author surface.**
#[doc(hidden)]
pub fn execute_sql(
    source: &str,
    values: &[Value],
    contract: &ResolvedContract,
    declared: &[TableUniqueConstraint],
    records: &mut BTreeMap<String, Vec<u8>>,
) -> yydb_types::Result<SqlOutcome> {
    let stmt = params::prepare_sql(source)?;
    let bound = params::bind_ordered(&stmt, values)?;
    match stmt.query_kind {
        QueryKind::Read => {
            let rows = params::execute_prepared(&stmt, &bound, Some(contract), records)?;
            Ok(SqlOutcome { rows, affected: 0 })
        }
        QueryKind::Write => {
            let outcome = params::execute_write_prepared(&stmt, &bound, contract, declared, records)?;
            Ok(SqlOutcome { rows: outcome.rows, affected: outcome.affected })
        }
    }
}
