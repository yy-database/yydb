//! YYDB public facade -- Rust embed `Connection` with **VOS as the only DDL and
//! query language**.
//!
//! ## Crate layering
//!
//! | Layer | Crate | Role |
//! |-------|-------|------|
//! | Bottom | `yydb-types` | `Error`, `Value`, CAS refs |
//! | Engine | `yydb-execution`, `yydb-udf`, `yydb-query` | Execution IR, UDF, query/DML |
//! | **Facade** | **`yydb`** | `Connection`, `wire`, re-exports |
//! | Transport | `yydb-client`, `yydb-server` | Remote client / serve loop |
//! | Bindings | `yydb-napi`, `yydb-pyo3`, ... | Language hosts only |
//!
//! **Embedded host:** register native [UDFs](udf) on [`Connection`]. Remote Rust
//! hosts use [`yydb-client`]. Lightweight TS hosts use `@yydb/yydb-client` against
//! `yydb-server`.
//!
//! Durable layout is still **one primary `.yydb` file**. Optional journal
//! sidecars `{path}-wal` / `{path}-shm` appear when
//! [`JournalMode::Wal`](journal::JournalMode::Wal) is enabled. Binary payloads
//! use the unified [`objects`] CAS (`objects/hash-2/*.bytes`) with optional
//! runtime hot/cold tiering — not a `.yydb-vec` sidecar.
//!
//! ```rust,no_run
//! use yydb::{Connection, OpenFlags, Result, Value};
//!
//! fn main() -> Result<()> {
//!     let conn = Connection::open_with_flags("app.yydb", OpenFlags::wal())?;
//!     conn.ensure_schema("table Project { @@id: uuid, title: utf8 }")?;
//!     conn.create_scalar("double", 1, |args| match args {
//!         [Value::I64(n)] => Ok(Value::I64(n * 2)),
//!         _ => Err(yydb::Error::Udf {
//!             name: "double".into(),
//!             message: "expected i64".into(),
//!         }),
//!     })?;
//!     conn.checkpoint()?;
//!     Ok(())
//! }
//! ```

#![deny(missing_docs)]

/// Shared types (`Error`, `Result`, `Value`, …).
pub mod types {
    pub use yydb_types::*;
}

/// Journal modes and `-wal` / `-shm` sidecar helpers.
pub mod journal;

/// Unified `objects/hash-2/*.bytes` CAS + hot/cold tiering.
pub mod objects;

/// Shared VOS schema contract (`vos` git @ `dev`).
pub mod schema;

/// UDF traits (`ScalarUdf`) and registry types (`UdfRegistry`, ...).
pub mod udf;

/// Phase 1 VOS query executor (read pipelines + insert writes).
pub mod query;

/// Common embed imports (`use yydb::prelude::*`).
pub mod prelude {
    pub use crate::{
        Batch, Connection, Error, JournalMode, ObjectRef, ObjectStore, OpenFlags, QueryRow, Result,
        SchemaVersion, Value,
    };
}

mod doctor;
mod lease;
mod refs;
mod ttl;
mod udf_bridge;

/// YY wire protocol (`YYDB`|`YYDS` + version digits `0000`…).
pub mod wire;

pub use journal::{JournalMode, OpenFlags};
pub use objects::ObjectStore;
pub use query::QueryRow;
pub use udf::ScalarUdf;

pub use yydb_execution as execution;
pub use yydb_query;
pub use yydb_types;
pub use yydb_udf;

pub use udf::{
    HostFunctionHandle, HostMicroDefinition, HostRuntimeAdapter, UdfError, UdfRegistry, UdfType,
    UdfValue,
};
/// Result alias for UDF host functions and registry operations.
pub type UdfResult<T = UdfValue> = yydb_udf::Result<T>;
pub use yydb_types::{
    ChunkManifest, CommitSequence, DoctorIssue, DoctorReport, DoctorSeverity, Error, EvictBudget,
    EvictReport, EvictionPolicy, HashAlgo, LeaseExpectation, LeaseToken, NamespaceQuota,
    NamespaceStats, ObjectKind, ObjectRef, ReclaimReport, RecordVersion, ReleaseOutcome, Result,
    SchemaVersion, Tier, Value, Vector, DEFAULT_CHUNK_SIZE, INLINE_BYTES_MAX,
};

/// VOS schema language facade (`git+https://github.com/voml/vos-language.git?branch=dev`).
pub use vos;

use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use journal::{
    append_snapshot_frame, ensure_wal_sidecars, remove_wal_sidecars, replay_wal_snapshots,
    shm_path, truncate_wal, wal_frame_count, wal_path,
};
use udf::{ClosureUdf, ScalarFn};
use udf_bridge::UdfSubsystem;

const MAGIC: &[u8] = b"YYDB\x03";
const LEGACY_MAGIC: &[u8] = b"YYDB\x01";
const CATALOG_MAGIC: &[u8] = b"YYDB\x02";

#[derive(Debug, Default, Clone)]
struct State {
    schema: Option<SchemaVersion>,
    catalog: Option<vos::ast::CatalogSnapshot>,
    resolved_contract: Option<vos::ResolvedContract>,
    records: BTreeMap<String, Vec<u8>>,
}

enum Backend {
    File {
        path: PathBuf,
        journal_mode: Mutex<JournalMode>,
    },
    Memory {
        state: Mutex<State>,
    },
}

/// Pending record mutations applied atomically by [`Batch::commit`].
#[derive(Debug, Default)]
pub struct Batch {
    pub(crate) changes: Vec<(String, Option<Vec<u8>>)>,
}

impl Batch {
    /// Create an empty batch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue an upsert for commit.
    pub fn put(&mut self, key: impl Into<String>, value: impl AsRef<[u8]>) {
        self.changes
            .push((key.into(), Some(value.as_ref().to_vec())));
    }

    /// Queue a delete for commit.
    pub fn delete(&mut self, key: impl Into<String>) {
        self.changes.push((key.into(), None));
    }

    /// Drop pending mutations without writing.
    pub fn abort(self) {}

    /// Queue a metadata record that references a CAS object.
    pub fn attach_object(&mut self, object: &ObjectRef, metadata_key: impl Into<String>) {
        self.put(metadata_key, refs::encode_object_ref(object));
    }

    /// Atomically apply queued mutations through `conn`.
    pub fn commit(self, conn: &Connection) -> Result<()> {
        conn.write_batch(&self.changes)
    }
}

/// A connection to a YYDB database (file-backed or in-memory).
///
/// Embedded `Connection` handle. DDL and query language are **VOS**.
/// UDFs are process-local and are **not** persisted in the `.yydb` file.
/// Binary payloads use [`ObjectStore`] (`objects/hash-2/*.bytes`).
pub struct Connection {
    backend: Backend,
    operation_lock: Mutex<()>,
    txn: Mutex<Option<State>>,
    udfs: Mutex<UdfSubsystem>,
    objects: ObjectStore,
}

impl Connection {
    /// Open (or create) a database at `path` with delete journal mode.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_flags(path, OpenFlags::new())
    }

    /// Open (or create) a database with explicit [`OpenFlags`].
    pub fn open_with_flags(path: impl AsRef<Path>, flags: OpenFlags) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            write_main(&path, &State::default())?;
        }
        match flags.journal_mode {
            JournalMode::Wal => ensure_wal_sidecars(&path)?,
            JournalMode::Delete => {
                // Leftover sidecars from a prior WAL session: fold then remove.
                if wal_path(&path).exists() || shm_path(&path).exists() {
                    let state = load_file_state(&path)?;
                    write_main(&path, &state)?;
                    remove_wal_sidecars(&path)?;
                }
            }
        }
        let connection = Self {
            operation_lock: Mutex::new(()),
            txn: Mutex::new(None),
            backend: Backend::File {
                path: path.clone(),
                journal_mode: Mutex::new(flags.journal_mode),
            },
            udfs: Mutex::new(UdfSubsystem::default()),
            objects: ObjectStore::open_beside_db(&path)?,
        };
        // Force recovery path once so a leftover WAL is applied.
        let _ = connection.read_state()?;
        connection.reconcile_leases_on_open()?;
        Ok(connection)
    }

    /// Open a private in-memory database (useful for tests).
    pub fn open_in_memory() -> Result<Self> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!("yydb-mem-objects-{nonce}"));
        Ok(Self {
            operation_lock: Mutex::new(()),
            txn: Mutex::new(None),
            backend: Backend::Memory {
                state: Mutex::new(State::default()),
            },
            udfs: Mutex::new(UdfSubsystem::default()),
            objects: ObjectStore::open_in_memory_root(root)?,
        })
    }

    /// Content-addressed object store (`<db>.objects/objects/…`).
    pub fn objects(&self) -> &ObjectStore {
        &self.objects
    }

    /// Filesystem path for file-backed connections; `None` for in-memory.
    pub fn path(&self) -> Option<&Path> {
        match &self.backend {
            Backend::File { path, .. } => Some(path.as_path()),
            Backend::Memory { .. } => None,
        }
    }

    /// Path of the `-wal` sidecar when file-backed.
    pub fn wal_path(&self) -> Option<PathBuf> {
        self.path().map(wal_path)
    }

    /// Path of the `-shm` sidecar when file-backed.
    pub fn shm_path(&self) -> Option<PathBuf> {
        self.path().map(shm_path)
    }

    /// Current journal mode (`delete` or `wal`). In-memory is always `delete`.
    pub fn journal_mode(&self) -> JournalMode {
        match &self.backend {
            Backend::File { journal_mode, .. } => *journal_mode
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            Backend::Memory { .. } => JournalMode::Delete,
        }
    }

    /// Switch journal mode. Enabling WAL creates sidecars; disabling WAL
    /// checkpoints then removes `{db}-wal` / `{db}-shm`.
    pub fn set_journal_mode(&self, mode: JournalMode) -> Result<()> {
        let Backend::File { path, journal_mode } = &self.backend else {
            return Ok(());
        };
        let mut slot = journal_mode
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *slot == mode {
            return Ok(());
        }
        match mode {
            JournalMode::Wal => {
                ensure_wal_sidecars(path)?;
            }
            JournalMode::Delete => {
                let state = load_file_state(path)?;
                write_main(path, &state)?;
                remove_wal_sidecars(path)?;
            }
        }
        *slot = mode;
        Ok(())
    }

    /// Fold WAL frames into the main file and truncate `-wal` / reset `-shm`.
    ///
    /// No-op when not in WAL mode or when there is nothing to fold.
    pub fn checkpoint(&self) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let Backend::File { path, journal_mode } = &self.backend else {
            return Ok(());
        };
        let mode = *journal_mode
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if mode != JournalMode::Wal {
            return Ok(());
        }
        let mut state = load_file_state(path)?;
        doctor::record_checkpoint(&mut state.records);
        write_main(path, &state)?;
        truncate_wal(path)?;
        Ok(())
    }

    /// Number of committed frames recorded in `-shm` (0 if absent).
    pub fn wal_frame_count(&self) -> Result<u32> {
        match &self.backend {
            Backend::File { path, .. } => wal_frame_count(path),
            Backend::Memory { .. } => Ok(0),
        }
    }

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
        match (state.resolved_contract, state.catalog) {
            (Some(contract), _) => {
                schema::execution_catalog_from_resolved_contract(&contract).map(Some)
            }
            (None, Some(catalog)) => schema::execution_catalog_from_snapshot(&catalog).map(Some),
            (None, None) => Ok(None),
        }
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

    /// Execute a Phase 1 VOS read pipeline (for example `User.filter(x => x.active).collect()`).
    pub fn query(&self, source: &str) -> Result<Vec<query::QueryRow>> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut state = self.active_state_unlocked()?;
        if let Some(catalog) = state.catalog.as_ref() {
            if let Some(rows) = query::try_insert_returning(source, catalog, &mut state.records)? {
                self.replace_active_state_unlocked(state)?;
                return Ok(rows);
            }
        }
        query::execute(source, state.catalog.as_ref(), &state.records)
    }

    /// Execute unit-valued VOS write programs (for example `User { … }.insert()`).
    pub fn execute(&self, source: &str) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        self.mutate_active_state_unlocked(|state| {
            let catalog = state.catalog.as_ref().ok_or_else(|| Error::Schema {
                message: "call ensure_schema before execute".into(),
            })?;
            let schema_document = state.schema.as_ref().map(|schema| schema.document.as_str());
            query::execute_write(source, catalog, schema_document, &mut state.records)
        })
    }

    /// Begin a data transaction on this connection.
    ///
    /// While open, `query` / `execute` / `upsert_row` read and write against an
    /// in-memory snapshot. [`Self::commit`] persists the snapshot, and
    /// [`Self::rollback`] discards it.
    pub fn begin(&self) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut slot = self
            .txn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.is_some() {
            return Err(Error::Schema {
                message: "transaction already open".into(),
            });
        }
        *slot = Some(self.read_state()?);
        Ok(())
    }

    /// Commit the open data transaction.
    pub fn commit(&self) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut slot = self
            .txn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(state) = slot.take() else {
            return Err(Error::Schema {
                message: "no open transaction".into(),
            });
        };
        drop(slot);
        self.write_state(&state)
    }

    /// Roll back the open data transaction.
    pub fn rollback(&self) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut slot = self
            .txn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.is_none() {
            return Err(Error::Schema {
                message: "no open transaction".into(),
            });
        }
        *slot = None;
        Ok(())
    }

    /// Whether a data transaction is open on this connection.
    pub fn in_transaction(&self) -> bool {
        self.txn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }

    /// Upsert one logical table row used by the Phase 1 query executor.
    pub fn upsert_row(
        &self,
        table: impl AsRef<str>,
        pk: impl AsRef<str>,
        row: query::QueryRow,
    ) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        self.mutate_active_state_unlocked(|state| {
            query::upsert_row(&mut state.records, table.as_ref(), pk.as_ref(), &row)
        })
    }

    /// Insert or replace a raw byte record keyed by `key`.
    ///
    /// Inline row storage is **allowed** but **not recommended** for large
    /// payloads — prefer [`Self::put_chunk`] / [`Self::put_file_chunked`] into
    /// the unified `objects/` CAS (see `INLINE_BYTES_MAX`).
    pub fn put(&self, key: impl Into<String>, value: impl AsRef<[u8]>) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut state = self.read_state()?;
        ttl::put_record(&mut state.records, &key.into(), value.as_ref())?;
        self.write_state(&state)
    }

    /// Store a record that expires after `ttl`.
    pub fn put_with_ttl(
        &self,
        key: impl Into<String>,
        value: impl AsRef<[u8]>,
        ttl: std::time::Duration,
    ) -> Result<RecordVersion> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut state = self.read_state()?;
        let version = ttl::put_with_ttl(
            &mut state.records,
            &key.into(),
            value.as_ref(),
            ttl.as_millis() as u64,
        )?;
        self.write_state(&state)?;
        Ok(version)
    }

    /// Configure quota limits for a namespace prefix such as `cache/response/`.
    pub fn set_namespace_quota(&self, namespace: &str, quota: NamespaceQuota) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut state = self.read_state()?;
        ttl::set_namespace_quota(&mut state.records, namespace, quota);
        self.write_state(&state)
    }

    /// Return byte and record usage for a namespace prefix.
    pub fn namespace_stats(&self, namespace: &str) -> Result<NamespaceStats> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        Ok(ttl::namespace_stats(&self.read_state()?.records, namespace))
    }

    /// Remove expired TTL records under `namespace_prefix` up to `budget`.
    pub fn evict_expired(
        &self,
        namespace_prefix: &str,
        budget: EvictBudget,
    ) -> Result<EvictReport> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut state = self.read_state()?;
        let report = ttl::evict_expired(&mut state.records, namespace_prefix, budget)?;
        self.write_state(&state)?;
        Ok(report)
    }

    /// Fetch a raw byte record by key.
    pub fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let state = self.read_state()?;
        if ttl::is_expired(&state.records, key) {
            return Ok(None);
        }
        Ok(state.records.get(key).cloned())
    }

    /// Commit a batch in one journal snapshot. `None` deletes the key.
    /// Operations are serialized on this connection, not across separate handles or processes.
    pub fn write_batch(&self, changes: &[(String, Option<Vec<u8>>)]) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut state = self.read_state()?;
        for (key, value) in changes {
            match value {
                Some(bytes) => {
                    state.records.insert(key.clone(), bytes.clone());
                }
                None => {
                    state.records.remove(key);
                }
            }
        }
        lease::bump_commit_sequence(&mut state.records);
        self.write_state(&state)
    }

    /// Replace or delete a key only if its current bytes match `expected`.
    /// `None` as expected requires an absent key. Atomic on a shared connection.
    pub fn compare_exchange(
        &self,
        key: &str,
        expected: Option<&[u8]>,
        replacement: Option<&[u8]>,
    ) -> Result<bool> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut state = self.read_state()?;
        if state.records.get(key).map(Vec::as_slice) != expected {
            return Ok(false);
        }
        match replacement {
            Some(bytes) => {
                state.records.insert(key.to_owned(), bytes.to_vec());
            }
            None => {
                state.records.remove(key);
            }
        }
        self.write_state(&state)?;
        Ok(true)
    }

    /// Scan keys in lexical order with an exclusive continuation key and a result limit.
    pub fn scan_prefix(
        &self,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(String, Vec<u8>)>> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let state = self.read_state()?;
        Ok(state
            .records
            .range(prefix.to_owned()..)
            .take_while(|(key, _)| key.starts_with(prefix))
            .filter(|(key, _)| after.map_or(true, |cursor| key.as_str() > cursor))
            .take(limit)
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect())
    }

    /// Claim a lease on `key` for `worker_id`.
    pub fn claim_lease(
        &self,
        key: &str,
        worker_id: &str,
        lease_duration: std::time::Duration,
        expectation: LeaseExpectation,
    ) -> Result<LeaseToken> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut state = self.read_state()?;
        let token = lease::claim_lease(
            &mut state.records,
            key,
            worker_id,
            lease_duration.as_millis() as u64,
            expectation,
        )?;
        self.write_state(&state)?;
        Ok(token)
    }

    /// Extend the lease deadline for an active token.
    pub fn renew_lease(&self, token: &LeaseToken, new_lease_until: u64) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut state = self.read_state()?;
        lease::renew_lease(&mut state.records, token, new_lease_until)?;
        self.write_state(&state)
    }

    /// Release a lease with a store-visible outcome.
    pub fn release_lease(&self, token: &LeaseToken, outcome: ReleaseOutcome) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut state = self.read_state()?;
        lease::release_lease(&mut state.records, token, outcome)?;
        self.write_state(&state)
    }

    /// Apply `batch` only when `token` matches the active claim.
    pub fn commit_with_lease(&self, token: &LeaseToken, batch: Batch) -> Result<CommitSequence> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut state = self.read_state()?;
        lease::verify_token(&state.records, token)?;
        for (key, value) in batch.changes {
            match value {
                Some(bytes) => {
                    state.records.insert(key, bytes);
                }
                None => {
                    state.records.remove(&key);
                }
            }
        }
        let sequence = lease::bump_commit_sequence(&mut state.records);
        lease::release_lease(&mut state.records, token, ReleaseOutcome::Requeue)?;
        self.write_state(&state)?;
        Ok(sequence)
    }

    /// Read the active lease deadline for `key`, if any.
    pub fn lease_until_millis(&self, key: &str) -> Result<Option<u64>> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        lease::lease_until(&self.read_state()?.records, key)
    }

    /// List CAS objects under `namespace_prefix` that have no committed metadata reference.
    pub fn scan_orphans(&self, namespace_prefix: &str) -> Result<Vec<ObjectRef>> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let state = self.read_state()?;
        let referenced = refs::referenced_hashes(&state.records, namespace_prefix);
        Ok(self
            .objects
            .list_objects()?
            .into_iter()
            .filter(|object| !referenced.contains(&object.hash))
            .collect())
    }

    /// Run read-only consistency probes against this database.
    pub fn doctor(&self) -> Result<DoctorReport> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let state = self.read_state()?;
        doctor::diagnose(&state.records, &self.objects, self.path(), self.wal_path())
    }

    /// Delete orphan objects that are still unreferenced after a fresh scan.
    pub fn reclaim_orphans(&self, objects: &[ObjectRef]) -> Result<ReclaimReport> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let state = self.read_state()?;
        let referenced = refs::referenced_hashes(&state.records, "");
        let mut report = ReclaimReport::default();
        for object in objects {
            if referenced.contains(&object.hash) {
                continue;
            }
            self.objects.remove_object(object)?;
            report.reclaimed_objects += 1;
        }
        Ok(report)
    }

    /// Store one CAS object at `objects/hash-2/<hash>.bytes`.
    pub fn put_chunk(&self, kind: ObjectKind, bytes: &[u8]) -> Result<ObjectRef> {
        self.objects.put_chunk(kind, bytes)
    }

    /// Store a vector payload in CAS (`ObjectKind::VectorPayload`).
    pub fn put_vector(&self, vector: &Vector) -> Result<ObjectRef> {
        self.objects.put_vector(vector)
    }

    /// Load a vector payload from CAS.
    pub fn get_vector(&self, object: &ObjectRef) -> Result<Vector> {
        self.objects.get_vector(object)
    }

    /// Read CAS bytes (fault-in to hot tier when needed).
    pub fn get_object(&self, object: &ObjectRef) -> Result<Arc<[u8]>> {
        self.objects.get_object(object)
    }

    /// Chunk a logical file into CAS objects.
    pub fn put_file_chunked(
        &self,
        reader: impl std::io::Read,
        chunk_size: usize,
    ) -> Result<ChunkManifest> {
        self.objects.put_file_chunked(reader, chunk_size)
    }

    /// Range-read a chunked file without assembling the whole payload.
    pub fn read_file_range(
        &self,
        manifest: &ChunkManifest,
        offset: u64,
        len: usize,
    ) -> Result<Vec<u8>> {
        self.objects.read_file_range(manifest, offset, len)
    }

    /// Pin an object in the hot tier.
    pub fn pin_object(&self, object: &ObjectRef) -> Result<Tier> {
        self.objects.pin_object(object)
    }

    /// Evict an object from the hot tier (disk CAS remains).
    pub fn evict_object(&self, object: &ObjectRef) -> Result<Tier> {
        self.objects.evict_object(object)
    }

    /// Current hot/cold hint for an object.
    pub fn object_tier(&self, object: &ObjectRef) -> Tier {
        self.objects.tier_of(object)
    }

    /// Register a Rust scalar UDF with a fixed arity.
    ///
    /// Replaces any previous registration with the same `name`. UDFs live only
    /// in this process; reopen the file and they are gone until re-registered.
    pub fn create_scalar<F>(&self, name: &str, n_args: usize, func: F) -> Result<()>
    where
        F: Fn(&[Value]) -> Result<Value> + Send + Sync + 'static,
    {
        self.create_scalar_versioned(name, 1, n_args, func)
    }

    /// Register a versioned Rust scalar UDF with a fixed arity.
    pub fn create_scalar_versioned<F>(
        &self,
        name: &str,
        version: u32,
        n_args: usize,
        func: F,
    ) -> Result<()>
    where
        F: Fn(&[Value]) -> Result<Value> + Send + Sync + 'static,
    {
        self.create_scalar_variadic_versioned(name, version, Some(n_args), func)
    }

    /// Register a Rust scalar UDF; `None` arity accepts any argument count.
    pub fn create_scalar_variadic<F>(&self, name: &str, arity: Option<usize>, func: F) -> Result<()>
    where
        F: Fn(&[Value]) -> Result<Value> + Send + Sync + 'static,
    {
        self.create_scalar_variadic_versioned(name, 1, arity, func)
    }

    /// Register a versioned Rust scalar UDF with optional fixed arity.
    pub fn create_scalar_variadic_versioned<F>(
        &self,
        name: &str,
        version: u32,
        arity: Option<usize>,
        func: F,
    ) -> Result<()>
    where
        F: Fn(&[Value]) -> Result<Value> + Send + Sync + 'static,
    {
        let boxed: Arc<ScalarFn> = Arc::new(func);
        if matches!(arity, Some(1) | Some(2)) {
            return self
                .udfs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .register_closure_scalar(name, version, arity, boxed);
        }
        let udf: Arc<dyn ScalarUdf> = Arc::new(ClosureUdf::new(arity, boxed));
        self.register_scalar_versioned(name, version, udf)
    }

    /// Register an object-safe [`ScalarUdf`] at version `1`.
    pub fn register_scalar(&self, name: &str, udf: Arc<dyn ScalarUdf>) -> Result<()> {
        self.register_scalar_versioned(name, 1, udf)
    }

    /// Register a validated local read-only execution body with its identity and version.
    pub fn register_execution_udf(&self, body: execution::ValidatedUdf) -> Result<()> {
        self.udfs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .register_execution_udf(body)
    }

    /// Parse and register a VOS-authored local scalar UDF.
    ///
    /// The source is parsed through the VOS facade, which uses Oak for the
    /// language surface. Only the validated local scalar subset is lowered by
    /// this embedded binder. The body is not persisted in the `.yydb` file.
    pub fn register_vos_scalar(&self, source: &str) -> Result<()> {
        self.register_vos_scalar_versioned(source, 1)
    }

    /// Parse and register a versioned VOS-authored local scalar UDF.
    pub fn register_vos_scalar_versioned(&self, source: &str, version: u32) -> Result<()> {
        self.udfs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .register_vos_scalar(source, version)
    }

    /// Parse and register a VOS-authored catalog macro UDF.
    ///
    /// The lowered body is registered in the connection catalog layer. Persistence
    /// into the `.yydb` file is not implemented yet.
    pub fn register_vos_macro(&self, source: &str) -> Result<()> {
        self.register_vos_macro_versioned(source, 1)
    }

    /// Parse and register a versioned VOS-authored catalog macro UDF.
    pub fn register_vos_macro_versioned(&self, source: &str, version: u32) -> Result<()> {
        self.udfs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .register_vos_macro(source, version)
    }

    /// Install the process-local host runtime adapter used by [`register_host_micro`].
    pub fn set_host_adapter(&self, adapter: Arc<dyn HostRuntimeAdapter>) {
        self.udfs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_host_adapter(adapter);
    }

    /// Register a session-local host micro through the host runtime adapter.
    pub fn register_host_micro(&self, definition: HostMicroDefinition) -> Result<()> {
        self.udfs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .register_host_micro(definition)
    }

    /// Register an object-safe [`ScalarUdf`] at an explicit version.
    pub fn register_scalar_versioned(
        &self,
        name: &str,
        version: u32,
        udf: Arc<dyn ScalarUdf>,
    ) -> Result<()> {
        self.udfs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .register_scalar_versioned(name, version, udf)
    }

    /// Remove a previously registered scalar UDF.
    pub fn remove_scalar(&self, name: &str) -> Result<()> {
        self.udfs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove_scalar(name)
    }

    /// Names of scalar UDFs registered on this connection.
    pub fn list_scalars(&self) -> Vec<String> {
        self.udfs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .list_scalars()
    }

    /// Invoke a registered scalar UDF at version `1`.
    pub fn call_scalar(&self, name: &str, args: &[Value]) -> Result<Value> {
        self.call_scalar_version(name, 1, args)
    }

    /// Invoke a registered scalar UDF at an explicit version.
    pub fn call_scalar_version(&self, name: &str, version: u32, args: &[Value]) -> Result<Value> {
        self.udfs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .call_scalar_version(name, version, args)
    }

    fn reconcile_leases_on_open(&self) -> Result<()> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        self.mutate_active_state_unlocked(|state| {
            lease::reconcile_expired_leases(&mut state.records)?;
            Ok(())
        })
    }

    fn active_state_unlocked(&self) -> Result<State> {
        if let Some(state) = self
            .txn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            Ok(state.clone())
        } else {
            self.read_state()
        }
    }

    fn replace_active_state_unlocked(&self, state: State) -> Result<()> {
        let mut slot = self
            .txn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.is_some() {
            *slot = Some(state);
            Ok(())
        } else {
            drop(slot);
            self.write_state(&state)
        }
    }

    fn mutate_active_state_unlocked<F>(&self, f: F) -> Result<()>
    where
        F: FnOnce(&mut State) -> Result<()>,
    {
        let mut state = self.active_state_unlocked()?;
        f(&mut state)?;
        self.replace_active_state_unlocked(state)
    }

    fn read_state(&self) -> Result<State> {
        match &self.backend {
            Backend::File { path, .. } => load_file_state(path),
            Backend::Memory { state } => Ok(state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()),
        }
    }

    fn write_state(&self, state: &State) -> Result<()> {
        match &self.backend {
            Backend::File { path, journal_mode } => {
                let mode = *journal_mode
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                match mode {
                    JournalMode::Delete => write_main(path, state),
                    JournalMode::Wal => {
                        let payload = encode(state)?;
                        append_snapshot_frame(path, &payload)
                    }
                }
            }
            Backend::Memory { state: slot } => {
                *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = state.clone();
                Ok(())
            }
        }
    }
}

/// Library version string (Cargo package version).
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn load_file_state(path: &Path) -> Result<State> {
    let main = fs::read(path)?;
    let merged = replay_wal_snapshots(path, main)?;
    decode(&merged)
}

fn write_main(path: &Path, state: &State) -> Result<()> {
    let bytes = encode(state)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temp = path.with_file_name(format!(
        ".{}.yydb-tmp-{}-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("database"),
        std::process::id(),
        nonce
    ));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        replace_main_file(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(not(windows))]
fn replace_main_file(temp: &Path, destination: &Path) -> Result<()> {
    fs::rename(temp, destination).map_err(Error::Io)
}

#[cfg(windows)]
fn replace_main_file(temp: &Path, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    let temp_wide = wide(temp);
    let destination_wide = wide(destination);
    let result = unsafe {
        if destination.exists() {
            ReplaceFileW(
                destination_wide.as_ptr(),
                temp_wide.as_ptr(),
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } else {
            MoveFileExW(
                temp_wide.as_ptr(),
                destination_wide.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
    };
    if result == 0 {
        Err(Error::Io(std::io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

fn encode(state: &State) -> Result<Vec<u8>> {
    let legacy = state.schema.is_some() && state.catalog.is_none();
    let mut bytes = if legacy {
        LEGACY_MAGIC.to_vec()
    } else if state.resolved_contract.is_some() {
        MAGIC.to_vec()
    } else {
        CATALOG_MAGIC.to_vec()
    };
    match &state.schema {
        Some(schema) => {
            bytes.push(1);
            bytes.extend(schema.version.to_le_bytes());
            write_bytes(&mut bytes, schema.document.as_bytes())?;
        }
        None => bytes.push(0),
    }
    write_u32(&mut bytes, state.records.len())?;
    for (key, value) in &state.records {
        write_bytes(&mut bytes, key.as_bytes())?;
        write_bytes(&mut bytes, value)?;
    }
    if legacy {
        return Ok(bytes);
    }
    match &state.catalog {
        Some(catalog) => {
            bytes.push(1);
            let encoded = serde_json::to_vec(catalog)
                .map_err(|_| Error::Corrupt("catalog serialization failed"))?;
            write_bytes(&mut bytes, &encoded)?;
        }
        None => bytes.push(0),
    }
    if let Some(contract) = &state.resolved_contract {
        bytes.push(1);
        let encoded = serde_json::to_vec(contract)
            .map_err(|_| Error::Corrupt("resolved contract serialization failed"))?;
        write_bytes(&mut bytes, &encoded)?;
    }
    Ok(bytes)
}

fn decode(bytes: &[u8]) -> Result<State> {
    let mut cursor = 0;
    let header = take(bytes, &mut cursor, MAGIC.len())?;
    let legacy = header == LEGACY_MAGIC;
    let has_resolved_contract = header == MAGIC;
    let has_catalog = header == MAGIC || header == CATALOG_MAGIC;
    if !legacy && !has_catalog {
        return Err(Error::Corrupt("unknown file header"));
    }
    let schema = match take(bytes, &mut cursor, 1)? {
        [0] => None,
        [1] => Some(SchemaVersion {
            version: read_u32(bytes, &mut cursor)?,
            document: String::from_utf8(read_bytes(bytes, &mut cursor)?)
                .map_err(|_| Error::Corrupt("schema is not UTF-8"))?,
        }),
        _ => return Err(Error::Corrupt("unknown schema marker")),
    };
    let count = read_u32(bytes, &mut cursor)?;
    let mut records = BTreeMap::new();
    for _ in 0..count {
        let key = String::from_utf8(read_bytes(bytes, &mut cursor)?)
            .map_err(|_| Error::Corrupt("record key is not UTF-8"))?;
        records.insert(key, read_bytes(bytes, &mut cursor)?);
    }
    if has_catalog {
        let catalog = match take(bytes, &mut cursor, 1)? {
            [0] => None,
            [1] => Some(
                serde_json::from_slice(&read_bytes(bytes, &mut cursor)?)
                    .map_err(|_| Error::Corrupt("catalog is invalid JSON"))?,
            ),
            _ => return Err(Error::Corrupt("unknown catalog marker")),
        };
        let resolved_contract = if !has_resolved_contract {
            None
        } else {
            match take(bytes, &mut cursor, 1)? {
                [0] => None,
                [1] => Some(
                    vos::ResolvedContract::from_json(
                        &String::from_utf8(read_bytes(bytes, &mut cursor)?)
                            .map_err(|_| Error::Corrupt("resolved contract is not UTF-8"))?,
                    )
                    .map_err(|_| Error::Corrupt("resolved contract is invalid"))?,
                ),
                _ => return Err(Error::Corrupt("unknown resolved contract marker")),
            }
        };
        if cursor != bytes.len() {
            return Err(Error::Corrupt("trailing data"));
        }
        if schema.is_some() != catalog.is_some() {
            return Err(Error::Corrupt("schema and catalog presence differ"));
        }
        if has_resolved_contract && catalog.is_some() != resolved_contract.is_some() {
            return Err(Error::Corrupt(
                "resolved contract and catalog presence differ",
            ));
        }
        if let (Some(schema), Some(catalog), Some(contract)) =
            (&schema, &catalog, &resolved_contract)
        {
            let expected = schema::resolved_contract_for_catalog(&schema.document, catalog)?;
            if &expected != contract {
                return Err(Error::Corrupt(
                    "resolved contract does not match schema ledger",
                ));
            }
        } else if let (Some(schema), Some(catalog)) = (&schema, &catalog) {
            schema::validate_snapshot(&schema.document, catalog)?;
        }
        return Ok(State {
            schema,
            catalog,
            resolved_contract,
            records,
        });
    }
    if cursor != bytes.len() {
        return Err(Error::Corrupt("trailing data"));
    }
    Ok(State {
        schema,
        catalog: None,
        resolved_contract: None,
        records,
    })
}

fn write_u32(bytes: &mut Vec<u8>, value: usize) -> Result<()> {
    let value = u32::try_from(value).map_err(|_| Error::Corrupt("value exceeds 4 GiB"))?;
    bytes.extend(value.to_le_bytes());
    Ok(())
}

fn write_bytes(bytes: &mut Vec<u8>, value: &[u8]) -> Result<()> {
    write_u32(bytes, value.len())?;
    bytes.extend(value);
    Ok(())
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32> {
    let raw = take(bytes, cursor, 4)?;
    Ok(u32::from_le_bytes(
        raw.try_into().expect("requested exactly 4 bytes"),
    ))
}

fn read_bytes(bytes: &[u8], cursor: &mut usize) -> Result<Vec<u8>> {
    let length = usize::try_from(read_u32(bytes, cursor)?).expect("u32 fits usize");
    Ok(take(bytes, cursor, length)?.to_vec())
}

fn take<'a>(bytes: &'a [u8], cursor: &mut usize, length: usize) -> Result<&'a [u8]> {
    let end = cursor
        .checked_add(length)
        .ok_or(Error::Corrupt("length overflow"))?;
    let value = bytes
        .get(*cursor..end)
        .ok_or(Error::Corrupt("unexpected EOF"))?;
    *cursor = end;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("yydb-{label}-{nonce}.yydb"))
    }

    fn cleanup(path: &Path) {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(wal_path(path));
        let _ = fs::remove_file(shm_path(path));
    }

    #[test]
    fn stores_schema_and_records_in_one_reopenable_file() {
        let path = temp_db("reopen");
        let conn = Connection::open(&path).unwrap();
        conn.ensure_schema("table Project { @@id: uuid }").unwrap();
        conn.put("project/meta", b"Spark").unwrap();
        drop(conn);

        let reopened = Connection::open(&path).unwrap();
        assert_eq!(reopened.schema().unwrap().unwrap().version, 1);
        assert_eq!(reopened.record_count().unwrap(), 1);
        assert_eq!(
            reopened.get("project/meta").unwrap(),
            Some(b"Spark".to_vec())
        );
        cleanup(&path);
    }

    #[test]
    fn atomically_replaces_main_image_without_leaving_temp_files() {
        let path = temp_db("atomic");
        let conn = Connection::open(&path).unwrap();
        conn.ensure_schema("table Project { @@id: uuid }").unwrap();
        conn.put("project/meta", b"Spark").unwrap();
        drop(conn);

        let file_name = path.file_name().unwrap().to_string_lossy();
        let prefix = format!(".{file_name}.yydb-tmp-");
        let leftovers = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
            .count();
        assert_eq!(leftovers, 0);

        let reopened = Connection::open(&path).unwrap();
        assert_eq!(
            reopened.get("project/meta").unwrap(),
            Some(b"Spark".to_vec())
        );
        drop(reopened);
        cleanup(&path);
    }

    #[test]
    fn open_in_memory_roundtrip() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(conn.path().is_none());
        conn.ensure_schema("table Demo { @@id: uuid }").unwrap();
        conn.put("k", b"v").unwrap();
        assert_eq!(conn.get("k").unwrap(), Some(b"v".to_vec()));
        assert_eq!(conn.schema().unwrap().unwrap().version, 1);
        assert_eq!(conn.record_count().unwrap(), 1);
    }

    #[test]
    fn batch_compare_exchange_and_prefix_scan_are_atomic_on_connection() {
        let conn = Connection::open_in_memory().unwrap();
        conn.write_batch(&[
            ("frontier/a".into(), Some(b"queued".to_vec())),
            ("frontier/b".into(), Some(b"queued".to_vec())),
            ("other/c".into(), Some(b"ignored".to_vec())),
        ])
        .unwrap();
        assert_eq!(conn.scan_prefix("frontier/", None, 10).unwrap().len(), 2);
        assert!(conn
            .compare_exchange("frontier/a", Some(b"queued"), Some(b"claimed"))
            .unwrap());
        assert!(!conn
            .compare_exchange("frontier/a", Some(b"queued"), Some(b"stale"))
            .unwrap());
        assert_eq!(conn.get("frontier/a").unwrap(), Some(b"claimed".to_vec()));
    }

    #[test]
    fn rust_scalar_udf_create_and_call() {
        let conn = Connection::open_in_memory().unwrap();
        conn.create_scalar("double", 1, |args| match args {
            [Value::I64(n)] => Ok(Value::I64(n * 2)),
            _ => Err(Error::Udf {
                name: "double".into(),
                message: "expected i64".into(),
            }),
        })
        .unwrap();
        assert_eq!(
            conn.call_scalar("double", &[Value::I64(21)]).unwrap(),
            Value::I64(42)
        );
        assert!(matches!(
            conn.call_scalar("double", &[]),
            Err(Error::UdfArity {
                expected: 1,
                got: 0,
                ..
            })
        ));
        assert_eq!(conn.list_scalars(), vec!["double".to_owned()]);
        conn.remove_scalar("double").unwrap();
        assert!(matches!(
            conn.call_scalar("double", &[Value::I64(1)]),
            Err(Error::UdfNotFound { .. })
        ));
    }

    #[test]
    fn wal_and_shm_sidecars_recover_without_checkpoint() {
        let path = temp_db("wal");
        let conn = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
        assert_eq!(conn.journal_mode(), JournalMode::Wal);
        assert!(conn
            .wal_path()
            .unwrap()
            .to_string_lossy()
            .ends_with(".yydb-wal"));
        assert!(conn
            .shm_path()
            .unwrap()
            .to_string_lossy()
            .ends_with(".yydb-shm"));
        conn.ensure_schema("table T { @@id: uuid }").unwrap();
        conn.put("a", b"1").unwrap();
        conn.put("b", b"2").unwrap();
        assert!(conn.wal_frame_count().unwrap() >= 2);
        assert!(wal_path(&path).exists());
        assert!(shm_path(&path).exists());
        drop(conn);

        // Main file still has the empty/default image; recovery must come from WAL.
        let reopened = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
        assert_eq!(reopened.get("a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(reopened.get("b").unwrap(), Some(b"2".to_vec()));
        reopened.checkpoint().unwrap();
        assert_eq!(reopened.wal_frame_count().unwrap(), 0);
        drop(reopened);

        let after_ckpt = Connection::open(&path).unwrap();
        assert_eq!(after_ckpt.get("b").unwrap(), Some(b"2".to_vec()));
        cleanup(&path);
    }

    #[test]
    fn set_journal_mode_wal_to_delete_removes_sidecars() {
        let path = temp_db("mode");
        let conn = Connection::open(&path).unwrap();
        conn.set_journal_mode(JournalMode::Wal).unwrap();
        conn.put("k", b"v").unwrap();
        assert!(wal_path(&path).exists());
        conn.set_journal_mode(JournalMode::Delete).unwrap();
        assert!(!wal_path(&path).exists());
        assert!(!shm_path(&path).exists());
        assert_eq!(conn.get("k").unwrap(), Some(b"v".to_vec()));
        cleanup(&path);
    }

    #[test]
    fn cas_objects_hash2_path_and_hot_cold() {
        let conn = Connection::open_in_memory().unwrap();
        let object = conn.put_chunk(ObjectKind::Blob, b"hello-cas").unwrap();
        let path = conn.objects().path_for(&object);
        let path_s = path.to_string_lossy().replace('\\', "/");
        assert!(path_s.contains("/objects/"));
        assert!(path_s.ends_with(".bytes"));
        let hex = object.hash_hex();
        assert!(path_s.contains(&format!("/{}/{}", &hex[..2], hex)));
        assert_eq!(&*conn.get_object(&object).unwrap(), b"hello-cas");
        assert_eq!(conn.pin_object(&object).unwrap(), Tier::Hot);
        assert_eq!(conn.object_tier(&object), Tier::Hot);
        assert_eq!(conn.evict_object(&object).unwrap(), Tier::Cold);
        assert_eq!(&*conn.get_object(&object).unwrap(), b"hello-cas");
    }

    #[test]
    fn chunked_file_range_read() {
        let conn = Connection::open_in_memory().unwrap();
        let payload: Vec<u8> = (0..2000u16).map(|v| (v % 256) as u8).collect();
        let manifest = conn
            .put_file_chunked(std::io::Cursor::new(payload.clone()), 500)
            .unwrap();
        assert!(manifest.chunks.len() >= 4);
        assert_eq!(manifest.total_size, 2000);
        let mid = conn.read_file_range(&manifest, 500, 500).unwrap();
        assert_eq!(mid, payload[500..1000]);
        let tail = conn.read_file_range(&manifest, 1800, 500).unwrap();
        assert_eq!(tail, payload[1800..]);
    }

    #[test]
    fn vector_via_cas_roundtrip() {
        let conn = Connection::open_in_memory().unwrap();
        let vector = Vector::new(vec![1.0, 2.5, -3.0]).unwrap();
        let object = conn.put_vector(&vector).unwrap();
        assert_eq!(object.kind, ObjectKind::VectorPayload);
        let loaded = conn.get_vector(&object).unwrap();
        assert_eq!(loaded.dim, 3);
        assert_eq!(&*loaded.data, &*vector.data);
    }

    #[test]
    fn version_is_non_empty() {
        assert!(!version().is_empty());
    }
}
