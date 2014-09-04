//! VOS query and DML execution on the connection record map.

use crate::catalog as schema;
use crate::{Error, Result};
use yydb_query::QueryRow;

use super::Connection;

impl Connection {
    /// Execute a VOS read pipeline (for example `User.all().collect()`).
    ///
    /// Reads the active connection state, including an open transaction snapshot.
    /// Requires [`Self::ensure_schema`] first.
    pub fn query_vos(&self, source: &str) -> Result<Vec<QueryRow>> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let state = self.active_state_unlocked()?;
        let catalog = catalog_projection(&state)?;
        yydb_query::execute(source, Some(&catalog), &state.records)
    }

    /// Execute a VOS write program (inserts and schema macros).
    ///
    /// Mutates the active connection state. When a transaction is open, changes
    /// stay in the snapshot until [`Self::commit`] or [`Self::rollback`].
    pub fn execute_vos_write(&self, source: &str) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.mutate_active_state_unlocked(|state| {
            let catalog = catalog_projection(state)?;
            let schema_document = state
                .schema
                .as_ref()
                .map(|schema| schema.document.as_str());
            yydb_query::execute_write(source, &catalog, schema_document, &mut state.records)
        })
    }
}

fn catalog_projection(state: &super::State) -> Result<vos::types::SchemaProjection> {
    let document = state
        .schema
        .as_ref()
        .ok_or_else(|| Error::Schema {
            message: "VOS query requires ensure_schema before execution".into(),
        })?
        .document
        .as_str();
    schema::project_schema(document)
}
