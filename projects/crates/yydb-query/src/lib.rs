//! Phase 1 VOS query executor (read pipelines + insert writes).
//!
//! Sits above [`yydb-types`] and below the [`yydb`] facade. Storage hosts pass
//! their in-memory record map and VOS catalog snapshot into these entry points.

#![deny(missing_docs)]

mod dml;
mod exec;
mod lower;
mod ops;
mod resolve;
mod store;

use std::collections::BTreeMap;

use vos::types::SchemaProjection;

pub use ops::QueryRow;
pub use store::upsert_row;

/// Execute a VOS read pipeline against stored table rows.
pub fn execute(
    source: &str,
    catalog: Option<&SchemaProjection>,
    records: &BTreeMap<String, Vec<u8>>,
) -> yydb_types::Result<Vec<QueryRow>> {
    exec::execute(source, catalog, records)
}

/// Execute unit-valued VOS write programs (for example `User { … }.insert()`).
pub fn execute_write(
    source: &str,
    catalog: &SchemaProjection,
    schema_document: Option<&str>,
    records: &mut BTreeMap<String, Vec<u8>>,
) -> yydb_types::Result<()> {
    dml::execute(source, catalog, schema_document, records)
}

/// Run a single `Type::insert({ … })` program and return the stored row when recognized.
pub fn try_insert_returning(
    source: &str,
    catalog: &SchemaProjection,
    records: &mut BTreeMap<String, Vec<u8>>,
) -> yydb_types::Result<Option<Vec<QueryRow>>> {
    dml::try_insert_returning(source, catalog, records)
}
