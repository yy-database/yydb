//! VOS query and DML execution on the connection record map.

use std::collections::BTreeMap;

use crate::{Error, Result};
use yydb_query::{PreparedStatement, QueryRow, fingerprint as fp, prepare as prepare_source};
use yydb_types::Value;

use super::Connection;

impl Connection {
    /// Prepare a VOS read pipeline into a reusable query plan.
    ///
    /// The returned [`PreparedStatement`] carries the lowered op tree, named
    /// parameter descriptors, source fingerprint and — when a schema is
    /// already bound on this connection — the current schema's fingerprint
    /// and revision. Executions later use those two fields to detect schema
    /// migration without re-parsing.
    pub fn prepare_vos(&self, source: &str) -> Result<PreparedStatement> {
        let _guard = self.operation_lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stmt = prepare_source(source)?;
        let state = self.active_state_unlocked()?;
        if let Some(schema) = &state.schema {
            stmt.schema_fingerprint = fp(&schema.document);
            stmt.schema_revision = state.identity_history.as_ref().map(|h| h.revision).unwrap_or(0);
        }
        Ok(stmt)
    }

    /// Execute a previously prepared statement with named parameter bindings.
    ///
    /// Reads the active connection state, including an open transaction
    /// snapshot. Returns [`Error::QueryBind`] when `named` is missing required
    /// parameters, contains extraneous ones, or provides values that do not
    /// match the declared slot types.
    ///
    /// Returns [`Error::SchemaInvalidated`] when the active schema document
    /// has drifted from the fingerprint captured at prepare time. Re-prepare
    /// after schema migration.
    pub fn query_prepared(&self, stmt: &PreparedStatement, named: &BTreeMap<String, Value>) -> Result<Vec<QueryRow>> {
        let _guard = self.operation_lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let state = self.active_state_unlocked()?;

        // Schema-fingerprint gate: only when the statement was prepared with
        // a live schema context (non-empty fingerprint).
        if !stmt.schema_fingerprint.is_empty() {
            let Some(schema) = &state.schema
            else {
                return Err(Error::SchemaInvalidated {
                    expected_fingerprint: stmt.schema_fingerprint.clone(),
                    current_fingerprint: String::new(),
                    schema_revision: stmt.schema_revision,
                });
            };
            let current_fp = fp(&schema.document);
            if current_fp != stmt.schema_fingerprint {
                return Err(Error::SchemaInvalidated {
                    expected_fingerprint: stmt.schema_fingerprint.clone(),
                    current_fingerprint: current_fp,
                    schema_revision: stmt.schema_revision,
                });
            }
        }

        let contract = resolved_contract(&state)?;
        let values = yydb_query::bind_named(stmt, named)?;
        yydb_query::execute_prepared(stmt, &values, Some(&contract), &state.records)
    }

    /// Convenience wrapper: prepare → bind → execute in one call.
    ///
    /// Equivalent to [`Self::prepare_vos`] followed by [`Self::query_prepared`]
    /// on the same connection. Prefer this when you only intend to run the
    /// source once with a single parameter set; use the explicit
    /// prepare-then-bind path when you need to re-run the same source.
    pub fn query_vos_with_params(&self, source: &str, named: &BTreeMap<String, Value>) -> Result<Vec<QueryRow>> {
        let stmt = self.prepare_vos(source)?;
        self.query_prepared(&stmt, named)
    }

    /// Execute a VOS read pipeline with no bound parameters.
    ///
    /// Internally calls [`Self::prepare_vos`] followed by
    /// [`Self::query_prepared`] with an empty parameter map.
    ///
    /// Reads the active connection state, including an open transaction
    /// snapshot. Requires [`Self::ensure_schema`] first.
    pub fn query_vos(&self, source: &str) -> Result<Vec<QueryRow>> {
        self.query_vos_with_params(source, &BTreeMap::new())
    }

    /// Execute a VOS write program (inserts and schema macros).
    ///
    /// Mutates the active connection state. When a transaction is open, changes
    /// stay in the snapshot until [`Self::commit`] or [`Self::rollback`].
    pub fn execute_vos_write(&self, source: &str) -> Result<()> {
        let _guard = self.operation_lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        self.mutate_active_state_unlocked(|state| {
            let contract = resolved_contract(state)?;
            let schema_document = state.schema.as_ref().map(|schema| schema.document.as_str());
            yydb_query::execute_write(source, &contract, schema_document, &mut state.records)
        })
    }
}

fn resolved_contract(state: &super::State) -> Result<vos::ResolvedContract> {
    state
        .resolved_contract
        .clone()
        .ok_or_else(|| Error::Schema { message: "VOS query requires ensure_schema before execution".into() })
}
