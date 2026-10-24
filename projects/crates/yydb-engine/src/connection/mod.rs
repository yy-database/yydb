//! [`Connection`] open paths, storage backends, and state I/O.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::{
    Error, INLINE_BYTES_MAX, Result, SchemaVersion,
    journal::{OpenFlags, shm_path, wal_path},
    objects::ObjectStore,
    storage::snapshot,
    udf::UdfSubsystem,
};
use yydb_storage::{Database, snapshot_from_main_bytes, snapshot_to_main_bytes};

mod backup;
mod compact;
mod doctor;
mod journal;
mod kv;
mod lease;
mod objects_api;
mod query;
mod schema_api;
mod state;
mod transaction;
mod udf;

#[derive(Debug, Default, Clone)]
pub(crate) struct State {
    pub(crate) schema: Option<SchemaVersion>,
    pub(crate) identity_history: Option<vos::types::IdentityHistory>,
    pub(crate) resolved_contract: Option<vos::ResolvedContract>,
    pub(crate) records: BTreeMap<String, Vec<u8>>,
    pub(crate) unique_constraints: Vec<yydb_types::TableUniqueConstraint>,
}

pub(crate) enum Backend {
    File { path: PathBuf, database: Mutex<Database> },
    Memory { state: Mutex<State> },
}

pub(crate) fn is_yydx_main_path(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("yydx")
}

/// Stable database id for process-local in-memory pagers.
pub(crate) fn in_memory_database_id() -> [u8; 16] {
    [0xAA, 0xBB, 0xCC, 0xDD, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 1]
}

/// A connection to a YYDB database (file-backed or in-memory).
///
/// Embedded `Connection` handle. DDL and query language are **VOS**.
/// UDFs are process-local and are **not** persisted in the `.yydb` file.
/// Binary payloads use [`ObjectStore`](crate::ObjectStore) (`YYBB` `.blob` segments).
pub struct Connection {
    pub(crate) backend: std::sync::Arc<Backend>,
    pub(crate) operation_lock: std::sync::Arc<Mutex<()>>,
    pub(crate) txn: Mutex<Option<State>>,
    pub(crate) udfs: Mutex<UdfSubsystem>,
    pub(crate) objects: std::sync::Arc<ObjectStore>,
    pub(crate) closed: std::sync::Arc<AtomicBool>,
}

impl Connection {
    /// Open (or create) a database at `path` with delete journal mode.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_flags(path, OpenFlags::new())
    }

    /// Open (or create) a `.yydx` main file with sibling `<stem>-objects/` blob root.
    pub fn open_yydx(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if path.extension().and_then(|ext| ext.to_str()) != Some("yydx") {
            return Err(Error::Unsupported("open_yydx requires a .yydx main file path"));
        }
        Self::open(path)
    }

    /// Open (or create) a database with explicit [`OpenFlags`].
    pub fn open_with_flags(path: impl AsRef<Path>, flags: OpenFlags) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let database = Database::open(&path, flags.journal_mode)?;
        let connection = Self {
            operation_lock: std::sync::Arc::new(Mutex::new(())),
            txn: Mutex::new(None),
            backend: std::sync::Arc::new(Backend::File { path: path.clone(), database: Mutex::new(database) }),
            udfs: Mutex::new(UdfSubsystem::default()),
            objects: std::sync::Arc::new(if is_yydx_main_path(&path) {
                ObjectStore::open_yydx_blob_root(ObjectStore::yydx_objects_root(&path))?
            }
            else {
                ObjectStore::open_ephemeral()
            }),
            closed: std::sync::Arc::new(AtomicBool::new(false)),
        };
        // Force recovery path once so a leftover WAL is applied.
        let _ = connection.read_state()?;
        connection.reconcile_leases_on_open()?;
        Ok(connection)
    }

    /// Open a private in-memory database (useful for tests).
    pub fn open_in_memory() -> Result<Self> {
        Ok(Self {
            operation_lock: std::sync::Arc::new(Mutex::new(())),
            txn: Mutex::new(None),
            backend: std::sync::Arc::new(Backend::Memory { state: Mutex::new(State::default()) }),
            udfs: Mutex::new(UdfSubsystem::default()),
            objects: std::sync::Arc::new(ObjectStore::open_ephemeral()),
            closed: std::sync::Arc::new(AtomicBool::new(false)),
        })
    }

    /// Create a session over the same durable store as this connection.
    ///
    /// Sessions share the pager, writer lock and content-addressed object store,
    /// but keep an independent transaction slot, UDF registry and host adapter.
    /// Servers open the database once and hand every peer its own session so
    /// concurrent clients never reopen the file or clobber each other's state.
    pub fn session(&self) -> Self {
        Self {
            backend: std::sync::Arc::clone(&self.backend),
            operation_lock: std::sync::Arc::clone(&self.operation_lock),
            txn: Mutex::new(None),
            udfs: Mutex::new(UdfSubsystem::default()),
            objects: std::sync::Arc::clone(&self.objects),
            closed: std::sync::Arc::clone(&self.closed),
        }
    }

    /// Serialize the current database as a checkpointed `YDPG` main-file byte image.
    pub fn main_snapshot(&self) -> Result<Vec<u8>> {
        match self.backend.as_ref() {
            Backend::Memory { .. } => {
                let state = self.read_state()?;
                snapshot_to_main_bytes(&snapshot::snapshot_from_state(&state)?, in_memory_database_id(), 0x01)
            }
            Backend::File { path, .. } => {
                self.checkpoint()?;
                Ok(std::fs::read(path)?)
            }
        }
    }

    /// Replace the in-memory database from a `YDPG` main-file byte image.
    pub fn load_main_snapshot(&self, bytes: &[u8]) -> Result<()> {
        match self.backend.as_ref() {
            Backend::Memory { .. } => {
                let snapshot = snapshot_from_main_bytes(bytes)?;
                let state = snapshot::state_from_snapshot(&snapshot)?;
                self.write_state(&state)
            }
            Backend::File { .. } => Err(Error::Unsupported("load_main_snapshot is only supported on in-memory connections")),
        }
    }

    /// Content-addressed object store (in-process for `.yydb`; `.yydx` blob roots later).
    pub fn objects(&self) -> &ObjectStore {
        &self.objects
    }

    /// Filesystem path for file-backed connections; `None` for in-memory.
    pub fn path(&self) -> Option<&Path> {
        match self.backend.as_ref() {
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

    pub(crate) fn is_single_file_yydb(&self) -> bool {
        self.path().is_some_and(|path| !is_yydx_main_path(path))
    }

    pub(crate) fn guard_yydb_cas_payload(&self, len: usize) -> Result<()> {
        if self.is_single_file_yydb() && len > INLINE_BYTES_MAX {
            return Err(Error::Unsupported(
                "CAS payloads larger than INLINE_BYTES_MAX require a .yydx layout in single-file .yydb mode",
            ));
        }
        Ok(())
    }

    /// Mark this connection and all derived sessions as closed.
    ///
    /// Subsequent `prepare_vos`, `query_prepared` and other surface methods
    /// return [`Error::SessionClosed`]. The underlying storage backend stays
    /// alive for any `Arc` clones still held.
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    /// Whether this connection (or a parent it was derived from via
    /// [`Self::session`]) has been closed.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Internal pre-flight gate: returns `Err(SessionClosed)` when the
    /// connection is closed. Used by every surface method that touches
    /// prepared statements so a stale handle fails fast instead of
    /// silently operating on a half-closed store.
    pub(crate) fn guard_session_alive(&self) -> Result<()> {
        if self.is_closed() { Err(Error::SessionClosed) } else { Ok(()) }
    }
}
