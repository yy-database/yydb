//! Phase 1 VOS query executor (read pipelines + insert writes).
//!
//! Sits above [`yydb-types`] and below the [`yydb`] facade. Storage hosts pass
//! their in-memory record map and persisted [`ResolvedContract`] into these entry points.

#![deny(missing_docs)]

mod catalog;
mod dml;
mod exec;
mod lower;
mod ops;
mod params;
mod resolve;
mod store;

use std::collections::BTreeMap;

use vos::types::ResolvedContract;

pub use catalog::resolved_contract_from_document;
pub use ops::QueryRow;
pub use params::{BoundStatement, ParameterDescriptor, PreparedStatement, bind_named, execute_prepared, prepare};
pub use store::upsert_row;

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

/// Run a single `Type::insert({ … })` program and return the stored row when recognized.
pub fn try_insert_returning(
    source: &str,
    contract: &ResolvedContract,
    records: &mut BTreeMap<String, Vec<u8>>,
) -> yydb_types::Result<Option<Vec<QueryRow>>> {
    dml::try_insert_returning(source, contract, records)
}
