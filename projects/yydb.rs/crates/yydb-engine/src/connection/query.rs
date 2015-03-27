//! Query and DML execution on the connection record map.
//!
//! Two frontends reach the same executor: VOS (`prepare_vos`, `query_vos`,
//! `execute_vos_write`) and SQL (`prepare_sql`, `query_sql`, `execute_sql_write`).

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
        self.prepare_with(source, prepare_source)
    }

    /// Prepare a SQL statement into a reusable query plan.
    ///
    /// The plan is the same operator set VOS lowers into; only the front half
    /// differs. `?` placeholders are declared as `?1`, `?2`, … parameters and
    /// bind positionally through [`Self::query_sql_with_params`] and
    /// [`Self::execute_sql_write`].
    ///
    /// **Not part of the `@yydb/yydb` public author surface.** SQL remains an
    /// internal execution path; downstream authors should use the VOS frontend
    /// only.
    #[doc(hidden)]
    pub fn prepare_sql(&self, source: &str) -> Result<PreparedStatement> {
        self.prepare_with(source, yydb_query::prepare_sql)
    }

    fn prepare_with(&self, source: &str, frontend: fn(&str) -> Result<PreparedStatement>) -> Result<PreparedStatement> {
        self.guard_session_alive()?;
        let _guard = self.operation_lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        // The frontend already classifies ops and fills `query_kind` — the
        // engine path only adds schema fingerprint / revision on top.
        let mut stmt = frontend(source)?;
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
        let values = yydb_query::bind_named(stmt, named)?;
        self.run_prepared_read(stmt, &values)
    }

    /// Execute a previously prepared statement with positional bindings.
    ///
    /// Values are matched to parameters in declaration order, which is how SQL
    /// `?` placeholders bind.
    pub fn query_prepared_ordered(&self, stmt: &PreparedStatement, values: &[Value]) -> Result<Vec<QueryRow>> {
        let values = yydb_query::bind_ordered(stmt, values)?;
        self.run_prepared_read(stmt, &values)
    }

    fn run_prepared_read(&self, stmt: &PreparedStatement, values: &[Value]) -> Result<Vec<QueryRow>> {
        self.guard_session_alive()?;
        let _guard = self.operation_lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let state = self.active_state_unlocked()?;
        self.check_schema_drift(stmt, &state)?;

        let contract = resolved_contract(&state)?;
        yydb_query::execute_prepared(stmt, values, Some(&contract), &state.records)
    }

    fn check_schema_drift(&self, stmt: &PreparedStatement, state: &super::State) -> Result<()> {
        // Schema-fingerprint gate: only when the statement was prepared with
        // a live schema context (non-empty fingerprint).
        if stmt.schema_fingerprint.is_empty() {
            return Ok(());
        }
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
        Ok(())
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

    /// Execute a SQL read statement with positional parameter values.
    ///
    /// **Not part of the `@yydb/yydb` public author surface.**
    #[doc(hidden)]
    pub fn query_sql_with_params(&self, source: &str, values: &[Value]) -> Result<Vec<QueryRow>> {
        let stmt = self.prepare_sql(source)?;
        self.query_prepared_ordered(&stmt, values)
    }

    /// Execute a SQL read statement with no parameters.
    ///
    /// **Not part of the `@yydb/yydb` public author surface.**
    #[doc(hidden)]
    pub fn query_sql(&self, source: &str) -> Result<Vec<QueryRow>> {
        self.query_sql_with_params(source, &[])
    }

    /// Execute a VOS write program (inserts and schema macros).
    ///
    /// Mutates the active connection state. When a transaction is open, changes
    /// stay in the snapshot until [`Self::commit`] or [`Self::rollback`].
    pub fn execute_vos_write(&self, source: &str) -> Result<()> {
        self.guard_session_alive()?;
        let _guard = self.operation_lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        self.mutate_active_state_unlocked(|state| {
            let contract = resolved_contract(state)?;
            let schema_document = state.schema.as_ref().map(|schema| schema.document.as_str());
            yydb_query::execute_write_with_constraints(
                source,
                &contract,
                &state.unique_constraints,
                schema_document,
                &mut state.records,
            )
        })
    }

    /// Execute a SQL write statement (INSERT / UPDATE / DELETE) with positional
    /// parameter values. Returns the number of affected rows.
    ///
    /// Mutates the active connection state. When a transaction is open, changes
    /// stay in the snapshot until [`Self::commit`] or [`Self::rollback`].
    ///
    /// **Not part of the `@yydb/yydb` public author surface.**
    #[doc(hidden)]
    pub fn execute_sql_write(&self, source: &str, values: &[Value]) -> Result<usize> {
        let stmt = self.prepare_sql(source)?;
        let values = yydb_query::bind_ordered(&stmt, values)?;
        self.guard_session_alive()?;
        let _guard = self.operation_lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut affected = 0;
        self.mutate_active_state_unlocked(|state| {
            self.check_schema_drift(&stmt, state)?;
            let contract = resolved_contract(state)?;
            let outcome =
                yydb_query::execute_write_prepared(&stmt, &values, &contract, &state.unique_constraints, &mut state.records)?;
            affected = outcome.affected;
            Ok(())
        })?;
        Ok(affected)
    }
}

fn resolved_contract(state: &super::State) -> Result<vos::ResolvedContract> {
    state
        .resolved_contract
        .clone()
        .ok_or_else(|| Error::Schema { message: "query requires ensure_schema before execution".into() })
}
