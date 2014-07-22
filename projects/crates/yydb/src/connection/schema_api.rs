use crate::schema;
use crate::ttl;
use crate::{Error, Result, SchemaVersion};

use super::Connection;

impl Connection {
    /// Current database-truth schema, if any.
    pub fn schema(&self) -> Result<Option<SchemaVersion>> {
        Ok(self.read_state()?.schema)
    }

    /// Current persisted VOS identity history, if the database has one.
    pub fn identity_history(&self) -> Result<Option<vos::types::IdentityHistory>> {
        Ok(self.read_state()?.identity_history)
    }

    /// Current validated VOS resolved contract, if the database has one.
    pub fn resolved_contract(&self) -> Result<Option<vos::ResolvedContract>> {
        Ok(self.read_state()?.resolved_contract)
    }

    /// Bind local execution handles using persisted identities, never fresh source order.
    pub fn execution_catalog(&self) -> Result<Option<schema::ExecutionCatalog>> {
        let state = self.read_state()?;
        let Some(contract) = state.resolved_contract else {
            let Some(schema) = state.schema else {
                return Ok(None);
            };
            self.ensure_schema(&schema.document)?;
            let state = self.read_state()?;
            return state
                .resolved_contract
                .as_ref()
                .map(schema::execution_catalog_from_resolved_contract)
                .transpose();
        };
        schema::execution_catalog_from_resolved_contract(&contract).map(Some)
    }

    /// Number of stored key/value records.
    pub fn record_count(&self) -> Result<usize> {
        Ok(self
            .read_state()?
            .records
            .keys()
            .filter(|key| !ttl::is_reserved_key(key))
            .count())
    }

    /// Current persisted schema version, if any.
    pub fn schema_version(&self) -> Result<Option<u32>> {
        Ok(self.schema()?.map(|schema| schema.version))
    }

    /// Persist the database-truth VOS schema on first use, then require an exact
    /// document match on later calls.
    ///
    /// The initial version is [`schema::initial_version`] (`1`, or
    /// `// @yydb-schema-version: <n>` in leading comments). To change the schema
    /// document after persistence, call [`Self::migrate_schema`].
    pub fn ensure_schema(&self, document: &str) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let projection = schema::project_schema(document)?;
        let mut state = self.read_state()?;
        if let Some(current) = &state.schema {
            if current.document != document {
                return Err(Error::Schema {
                    message:
                        "schema document changed -- call migrate_schema to advance the version"
                            .into(),
                });
            }
            let resolved = rebind_identities(&state, &projection)?;
            if state.resolved_contract.is_none() {
                state.resolved_contract = Some(resolved);
                self.write_state(&state)?;
            }
            return Ok(());
        }
        let version = schema::initial_version(document);
        let manifest = schema::allocate_initial_manifest(&projection);
        let bound = schema::bind(&projection, &manifest)?;
        let history = schema::evolve(&schema::empty_history(), bound)?;
        state.resolved_contract = Some(schema::resolve_contract(&history.snapshot)?);
        state.identity_history = Some(history);
        state.schema = Some(SchemaVersion {
            version,
            document: document.to_owned(),
        });
        self.write_state(&state)
    }

    /// Evolve the persisted schema to the next version (`current + 1`).
    ///
    /// Existing type and field identities are preserved by exact name matching
    /// or by mappings in `renames`. When the document carries
    /// `// @yydb-schema-version: <n>`, `n` must equal the new version.
    pub fn migrate_schema(&self, document: &str, renames: &schema::RenameMap) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let projection = schema::project_schema(document)?;
        let mut state = self.read_state()?;
        let current = state.schema.as_ref().ok_or_else(|| Error::Schema {
            message: "schema migration requires an existing schema".into(),
        })?;
        let version = current.version + 1;
        schema::validate_migration_version(document, version)?;
        let history = state
            .identity_history
            .as_ref()
            .ok_or_else(|| Error::Schema {
                message: "initialize the identity ledger with ensure_schema before migration"
                    .into(),
            })?;
        let manifest = schema::migration_manifest(history, &projection, renames)?;
        let bound = schema::bind(&projection, &manifest)?;
        let evolved = schema::evolve(history, bound)?;
        let resolved_contract = schema::resolve_contract(&evolved.snapshot)?;
        state.schema = Some(SchemaVersion {
            version,
            document: document.to_owned(),
        });
        state.identity_history = Some(evolved);
        state.resolved_contract = Some(resolved_contract);
        self.write_state(&state)
    }
}

/// Reproject a document against persisted identities and verify the ledger.
///
/// Returns the resolved contract for the persisted snapshot. When the document
/// no longer matches the durable ledger this fails structurally without
/// rewriting any state.
fn rebind_identities(
    state: &super::State,
    projection: &vos::types::SchemaProjection,
) -> Result<vos::ResolvedContract> {
    let history = state.identity_history.as_ref().ok_or(Error::Corrupt(
        "catalog identity ledger does not match schema",
    ))?;
    let manifest = schema::manifest_from_snapshot(&history.snapshot);
    let bound = schema::bind(projection, &manifest)
        .map_err(|_| Error::Corrupt("catalog identity ledger does not match schema"))?;
    if bound != history.snapshot {
        return Err(Error::Corrupt(
            "catalog identity ledger does not match schema",
        ));
    }
    schema::resolve_contract(&bound)
}
