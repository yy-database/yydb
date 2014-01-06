//! YYDB public facade — Rust embed `Connection` with **VOS as the only DDL and
//! query language**.
//!
//! **Embedded host:** Rust only. Register native [UDFs](udf) on
//! [`Connection`]. Other languages should use the `yydb-client` crate against
//! `yydb serve`, not this embeddable library.
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
//!     conn.ensure_schema(1, "table Project { @@id: uuid, title: utf8 }")?;
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

/// Rust scalar UDF traits and registration helpers.
pub mod udf;

/// YY wire protocol (`YYDB`|`YYDS` + version digits `0000`…).
pub mod wire;

pub use journal::{JournalMode, OpenFlags};
pub use objects::ObjectStore;
pub use udf::ScalarUdf;
pub use yydb_types::{
    ChunkManifest, Error, HashAlgo, ObjectKind, ObjectRef, Result, SchemaVersion, Tier, Value,
    Vector, DEFAULT_CHUNK_SIZE, INLINE_BYTES_MAX,
};

/// VOS schema language facade (`git+https://github.com/voml/vos-language.git?branch=dev`).
pub use vos;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use journal::{
    append_snapshot_frame, ensure_wal_sidecars, remove_wal_sidecars, replay_wal_snapshots,
    shm_path, truncate_wal, wal_frame_count, wal_path,
};
use udf::{ClosureUdf, RegisteredUdf, ScalarFn};

const MAGIC: &[u8] = b"YYDB\x01";

#[derive(Debug, Default, Clone)]
struct State {
    schema: Option<SchemaVersion>,
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

/// A connection to a YYDB database (file-backed or in-memory).
///
/// Embedded `Connection` handle. DDL and query language are **VOS**.
/// UDFs are process-local and are **not** persisted in the `.yydb` file.
/// Binary payloads use [`ObjectStore`] (`objects/hash-2/*.bytes`).
pub struct Connection {
    backend: Backend,
    udfs: Mutex<BTreeMap<String, RegisteredUdf>>,
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
            backend: Backend::File {
                path: path.clone(),
                journal_mode: Mutex::new(flags.journal_mode),
            },
            udfs: Mutex::new(BTreeMap::new()),
            objects: ObjectStore::open_beside_db(&path)?,
        };
        // Force recovery path once so a leftover WAL is applied.
        let _ = connection.read_state()?;
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
            backend: Backend::Memory {
                state: Mutex::new(State::default()),
            },
            udfs: Mutex::new(BTreeMap::new()),
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
        let Backend::File { path, journal_mode } = &self.backend else {
            return Ok(());
        };
        let mode = *journal_mode
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if mode != JournalMode::Wal {
            return Ok(());
        }
        let state = load_file_state(path)?;
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

    /// Number of stored key/value records.
    pub fn record_count(&self) -> Result<usize> {
        Ok(self.read_state()?.records.len())
    }

    /// Stores the database truth schema when empty and rejects a mismatched
    /// version thereafter. `document` is **VOS** source (shared with
    /// `vos` git @ `dev`); it is validated before persistence. Schema migration
    /// is an explicit future operation.
    pub fn ensure_schema(&self, version: u32, document: &str) -> Result<()> {
        schema::validate_document(document)?;
        let mut state = self.read_state()?;
        match &state.schema {
            Some(schema) if schema.version != version => {
                return Err(Error::SchemaConflict {
                    expected: version,
                    found: schema.version,
                });
            }
            Some(_) => return Ok(()),
            None => {
                state.schema = Some(SchemaVersion {
                    version,
                    document: document.to_owned(),
                })
            }
        }
        self.write_state(&state)
    }

    /// Insert or replace a raw byte record keyed by `key`.
    ///
    /// Inline row storage is **allowed** but **not recommended** for large
    /// payloads — prefer [`Self::put_chunk`] / [`Self::put_file_chunked`] into
    /// the unified `objects/` CAS (see `INLINE_BYTES_MAX`).
    pub fn put(&self, key: impl Into<String>, value: impl AsRef<[u8]>) -> Result<()> {
        let mut state = self.read_state()?;
        state.records.insert(key.into(), value.as_ref().to_vec());
        self.write_state(&state)
    }

    /// Fetch a raw byte record by key.
    pub fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.read_state()?.records.get(key).cloned())
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
        self.create_scalar_variadic(name, Some(n_args), func)
    }

    /// Register a Rust scalar UDF; `None` arity accepts any argument count.
    pub fn create_scalar_variadic<F>(&self, name: &str, arity: Option<usize>, func: F) -> Result<()>
    where
        F: Fn(&[Value]) -> Result<Value> + Send + Sync + 'static,
    {
        let boxed: Arc<ScalarFn> = Arc::new(func);
        let udf: Arc<dyn ScalarUdf> = Arc::new(ClosureUdf::new(arity, boxed));
        self.register_scalar(name, udf)
    }

    /// Register an object-safe [`ScalarUdf`].
    pub fn register_scalar(&self, name: &str, udf: Arc<dyn ScalarUdf>) -> Result<()> {
        if name.is_empty() {
            return Err(Error::Udf {
                name: String::new(),
                message: "UDF name must not be empty".into(),
            });
        }
        self.udfs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(name.to_owned(), RegisteredUdf { udf });
        Ok(())
    }

    /// Remove a previously registered scalar UDF.
    pub fn remove_scalar(&self, name: &str) -> Result<()> {
        let removed = self
            .udfs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(name)
            .is_some();
        if removed {
            Ok(())
        } else {
            Err(Error::UdfNotFound {
                name: name.to_owned(),
            })
        }
    }

    /// Names of scalar UDFs registered on this connection.
    pub fn list_scalars(&self) -> Vec<String> {
        self.udfs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    /// Invoke a registered scalar UDF (also used by future query execution).
    pub fn call_scalar(&self, name: &str, args: &[Value]) -> Result<Value> {
        let guard = self
            .udfs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entry = guard.get(name).ok_or_else(|| Error::UdfNotFound {
            name: name.to_owned(),
        })?;
        if let Some(expected) = entry.udf.arity() {
            if args.len() != expected {
                return Err(Error::UdfArity {
                    name: name.to_owned(),
                    expected,
                    got: args.len(),
                });
            }
        }
        entry.udf.call(args).map_err(|error| match error {
            Error::Udf { .. }
            | Error::UdfArity { .. }
            | Error::UdfNotFound { .. }
            | Error::ObjectNotFound { .. }
            | Error::ObjectCorrupt { .. }
            | Error::Unsupported(_) => error,
            other => Error::Udf {
                name: name.to_owned(),
                message: other.to_string(),
            },
        })
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
    fs::write(path, encode(state)?).map_err(Error::Io)
}

fn encode(state: &State) -> Result<Vec<u8>> {
    let mut bytes = MAGIC.to_vec();
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
    Ok(bytes)
}

fn decode(bytes: &[u8]) -> Result<State> {
    let mut cursor = 0;
    if take(bytes, &mut cursor, MAGIC.len())? != MAGIC {
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
    if cursor != bytes.len() {
        return Err(Error::Corrupt("trailing data"));
    }
    Ok(State { schema, records })
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
        conn.ensure_schema(1, "table Project { id: uuid }").unwrap();
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
    fn open_in_memory_roundtrip() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(conn.path().is_none());
        conn.ensure_schema(2, "table Demo { @@id: uuid }").unwrap();
        conn.put("k", b"v").unwrap();
        assert_eq!(conn.get("k").unwrap(), Some(b"v".to_vec()));
        assert_eq!(conn.schema().unwrap().unwrap().version, 2);
        assert_eq!(conn.record_count().unwrap(), 1);
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
        conn.ensure_schema(1, "table T { @@id: uuid }").unwrap();
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
