use crate::schema;
use crate::ttl;
use crate::udf::ScalarUdf;
use crate::{Error, Result, SchemaVersion};

use super::Connection;

impl Connection {
    /// Current database-truth schema, if any.
    pub fn schema(&self) -> Result<Option<SchemaVersion>> {
        Ok(self.read_state()?.schema)
    }

    /// Current persisted VOS identity ledger, if the database has one.
    pub fn catalog_snapshot(&self) -> Result<Option<vos::ast::CatalogSnapshot>> {
        Ok(self.read_state()?.catalog)
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
        schema::validate_document(document)?;
        let mut state = self.read_state()?;
        if let Some(current) = &state.schema {
            if current.document != document {
                return Err(Error::Schema {
                    message:
                        "schema document changed -- call migrate_schema to advance the version"
                            .into(),
                });
            }
            if let Some(catalog) = &state.catalog {
                if state.resolved_contract.is_none() {
                    state.resolved_contract =
                        Some(schema::resolved_contract_for_catalog(document, catalog)?);
                    self.write_state(&state)?;
                }
                return Ok(());
            }
        }
        let version = state
            .schema
            .as_ref()
            .map(|schema| schema.version)
            .unwrap_or_else(|| schema::initial_version(document));
        let parsed =
            vos::parser::parse_document(document).map_err(|diagnostics| Error::Schema {
                message: diagnostics.to_string(),
            })?;
        let catalog =
            vos::catalog_from_document(&parsed).map_err(|message| Error::Schema { message })?;
        state.resolved_contract = Some(schema::resolved_contract_for_catalog(document, &catalog)?);
        state.catalog = Some(catalog);
        state.schema = Some(SchemaVersion {
            version,
            document: document.to_owned(),
        });
        self.write_state(&state)
    }

    /// Evolve the persisted schema to the next version (`current + 1`).
    ///
    /// Existing type and field identities are preserved only by exact name
    /// matching or by mappings in `renames`. When the document carries
    /// `// @yydb-schema-version: <n>`, `n` must equal the new version.
    pub fn migrate_schema(&self, document: &str, renames: &vos::ast::RenameMap) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        schema::validate_document(document)?;
        let parsed =
            vos::parser::parse_document(document).map_err(|diagnostics| Error::Schema {
                message: diagnostics.to_string(),
            })?;
        let mut state = self.read_state()?;
        let current = state.schema.as_ref().ok_or_else(|| Error::Schema {
            message: "schema migration requires an existing schema".into(),
        })?;
        let expected = current.version;
        let version = expected + 1;
        schema::validate_migration_version(document, version)?;
        let previous_catalog = state.catalog.as_ref().ok_or_else(|| Error::Schema {
            message: "initialize the identity ledger with ensure_schema before migration".into(),
        })?;
        let catalog = vos::evolve_catalog(previous_catalog, &parsed, renames)
            .map_err(|message| Error::Schema { message })?;
        let resolved_contract = schema::resolved_contract_for_catalog(document, &catalog)?;
        state.schema = Some(SchemaVersion {
            version,
            document: document.to_owned(),
        });
        state.catalog = Some(catalog);
        state.resolved_contract = Some(resolved_contract);
        self.write_state(&state)
    }
}
