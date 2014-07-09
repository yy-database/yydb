//! [`Connection`] open paths, storage backends, and state I/O.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::file_lock::WriterLock;
use crate::format_v0;
use crate::journal::{shm_path, wal_path, JournalMode, OpenFlags};
use crate::objects::ObjectStore;
use crate::udf_bridge::UdfSubsystem;
use crate::INLINE_BYTES_MAX;
use crate::{Error, Result, SchemaVersion};
use yydb_format::{
    main_bytes_from_memory_pager, memory_pager_from_main_bytes, FilePager, MemoryPager,
};

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
mod udf;

#[derive(Debug, Default, Clone)]
pub(crate) struct State {
    pub(crate) schema: Option<SchemaVersion>,
    pub(crate) catalog: Option<vos::ast::CatalogSnapshot>,
    pub(crate) resolved_contract: Option<vos::ResolvedContract>,
    pub(crate) records: BTreeMap<String, Vec<u8>>,
}

pub(crate) enum Backend {
    File {
        path: PathBuf,
        #[allow(dead_code)]
        writer_lock: WriterLock,
        pager: Mutex<FilePager>,
        journal_mode: Mutex<JournalMode>,
    },
    Memory {
        state: Mutex<State>,
    },
}

pub(crate) fn is_yydx_main_path(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("yydx")
}

pub(crate) fn main_snapshot_database_id() -> [u8; 16] {
    [
        0xAA, 0xBB, 0xCC, 0xDD, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 1,
    ]
}

/// A connection to a YYDB database (file-backed or in-memory).
///
/// Embedded `Connection` handle. DDL and query language are **VOS**.
/// UDFs are process-local and are **not** persisted in the `.yydb` file.
/// Binary payloads use [`ObjectStore`](crate::ObjectStore) (`YYBB` `.blob` segments).
pub struct Connection {
    pub(crate) backend: Backend,
    pub(crate) operation_lock: Mutex<()>,
    pub(crate) txn: Mutex<Option<State>>,
    pub(crate) udfs: Mutex<UdfSubsystem>,
    pub(crate) objects: ObjectStore,
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
            return Err(Error::Unsupported(
                "open_yydx requires a .yydx main file path",
            ));
        }
        Self::open(path)
    }

    /// Open (or create) a database with explicit [`OpenFlags`].
    pub fn open_with_flags(path: impl AsRef<Path>, flags: OpenFlags) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if path.exists() && !format_v0::file_is_ydpg(&path)? {
            return Err(Error::Corrupt("main file is not YDPG format v0"));
        }
        let writer_lock = WriterLock::acquire(&path)?;
        let pager = format_v0::open_pager(&path, flags.journal_mode)?;
        let connection = Self {
            operation_lock: Mutex::new(()),
            txn: Mutex::new(None),
            backend: Backend::File {
                path: path.clone(),
                writer_lock,
                pager: Mutex::new(pager),
                journal_mode: Mutex::new(flags.journal_mode),
            },
            udfs: Mutex::new(UdfSubsystem::default()),
            objects: if is_yydx_main_path(&path) {
                ObjectStore::open_yydx_blob_root(ObjectStore::yydx_objects_root(&path))?
            } else {
                ObjectStore::open_ephemeral()
            },
        };
        // Force recovery path once so a leftover WAL is applied.
        let _ = connection.read_state()?;
        connection.reconcile_leases_on_open()?;
        Ok(connection)
    }

    /// Open a private in-memory database (useful for tests).
    pub fn open_in_memory() -> Result<Self> {
        Ok(Self {
            operation_lock: Mutex::new(()),
            txn: Mutex::new(None),
            backend: Backend::Memory {
                state: Mutex::new(State::default()),
            },
            udfs: Mutex::new(UdfSubsystem::default()),
            objects: ObjectStore::open_ephemeral(),
        })
    }

    /// Serialize the current database as a checkpointed `YDPG` main-file byte image.
    pub fn main_snapshot(&self) -> Result<Vec<u8>> {
        match &self.backend {
            Backend::Memory { .. } => {
                let state = self.read_state()?;
                let mut pager = MemoryPager::new_empty(main_snapshot_database_id(), 0x01);
                format_v0::write_state_to_memory_pager(&mut pager, &state)?;
                main_bytes_from_memory_pager(&pager)
            }
            Backend::File { path, .. } => {
                self.checkpoint()?;
                Ok(fs::read(path)?)
            }
        }
    }

    /// Replace the in-memory database from a `YDPG` main-file byte image.
    pub fn load_main_snapshot(&self, bytes: &[u8]) -> Result<()> {
        match &self.backend {
            Backend::Memory { .. } => {
                let mut pager = memory_pager_from_main_bytes(bytes)?;
                let state = format_v0::read_state_from_memory_pager(&mut pager)?;
                self.write_state(&state)
            }
            Backend::File { .. } => Err(Error::Unsupported(
                "load_main_snapshot is only supported on in-memory connections",
            )),
        }
    }

    /// Content-addressed object store (in-process for `.yydb`; `.yydx` blob roots later).
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
}
