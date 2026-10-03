//! Phase 1 VOS read query executor.

mod exec;
mod lower;
mod ops;
mod store;

pub use ops::QueryRow;
pub use store::upsert_row;

/// Execute a VOS read pipeline against stored table rows.
pub fn execute(source: &str, records: &std::collections::BTreeMap<String, Vec<u8>>) -> yydb_types::Result<Vec<QueryRow>> {
    exec::execute(source, records)
}
