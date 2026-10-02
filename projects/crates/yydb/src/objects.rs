//! Unified content-addressed store: `objects/hash-2/<hash>.bytes`.
//!
//! Large logical files are **chunked** manifests; vectors and ANN segments use
//! the same path layout. Hot/cold tiering is an in-process cache over this CAS.

use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use yydb_types::{
    ChunkManifest, Error, HashAlgo, ObjectKind, ObjectRef, Result, Tier, Vector,
    DEFAULT_CHUNK_SIZE, INLINE_BYTES_MAX,
};

/// Soft inline size guidance (re-exported for callers).
pub use yydb_types::INLINE_BYTES_MAX as INLINE_BYTES_SOFT_MAX;

/// Content-addressed object store rooted beside a `.yydb` file.
pub struct ObjectStore {
    /// `<db>.objects` directory (contains the `objects/` CAS tree).
    root: PathBuf,
    hot: Mutex<HashMap<[u8; 32], Arc<[u8]>>>,
    pinned: Mutex<HashSet<[u8; 32]>>,
}

impl ObjectStore {
    /// Open or create the store next to `db_path` (`app.yydb` → `app.yydb.objects`).
    pub fn open_beside_db(db_path: &Path) -> Result<Self> {
        let mut os = db_path.as_os_str().to_owned();
        os.push(".objects");
        let root = PathBuf::from(os);
        fs::create_dir_all(root.join("objects"))?;
        Ok(Self {
            root,
            hot: Mutex::new(HashMap::new()),
            pinned: Mutex::new(HashSet::new()),
        })
    }

    /// In-memory-only store (tests); still uses a temp directory if `root` is set.
    pub fn open_in_memory_root(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(root.join("objects"))?;
        Ok(Self {
            root,
            hot: Mutex::new(HashMap::new()),
            pinned: Mutex::new(HashSet::new()),
        })
    }

    /// Absolute `<db>.objects` root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Relative CAS directory `…/objects`.
    pub fn cas_root(&self) -> PathBuf {
        self.root.join("objects")
    }

    /// Path for a digest: `objects/<hash-2>/<hash>.bytes`.
    pub fn path_for_hash(&self, hash_hex: &str) -> PathBuf {
        let prefix: String = hash_hex.chars().take(2).collect();
        self.cas_root()
            .join(prefix)
            .join(format!("{hash_hex}.bytes"))
    }

    /// Path for an [`ObjectRef`].
    pub fn path_for(&self, object: &ObjectRef) -> PathBuf {
        self.path_for_hash(&object.hash_hex())
    }

    /// Whether inline storage is within the soft recommendation.
    pub fn inline_recommended(len: usize) -> bool {
        len <= INLINE_BYTES_MAX
    }

    /// Write one CAS chunk / object; returns its [`ObjectRef`].
    pub fn put_chunk(&self, kind: ObjectKind, bytes: &[u8]) -> Result<ObjectRef> {
        let hash = blake3::hash(bytes);
        let hash_bytes = *hash.as_bytes();
        let object = ObjectRef {
            algo: HashAlgo::Blake3,
            hash: hash_bytes,
            size: bytes.len() as u64,
            kind,
        };
        let path = self.path_for(&object);
        if !path.exists() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = File::create(&path)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        self.hot
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(hash_bytes, Arc::<[u8]>::from(bytes.to_vec()));
        Ok(object)
    }

    /// Read an object, faulting from disk into the hot set when needed.
    pub fn get_object(&self, object: &ObjectRef) -> Result<Arc<[u8]>> {
        {
            let hot = self.hot.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(bytes) = hot.get(&object.hash) {
                return Ok(Arc::clone(bytes));
            }
        }
        let path = self.path_for(object);
        if !path.exists() {
            return Err(Error::ObjectNotFound {
                hash_hex: object.hash_hex(),
            });
        }
        let bytes = fs::read(&path)?;
        if bytes.len() as u64 != object.size {
            return Err(Error::ObjectCorrupt {
                message: format!(
                    "size mismatch for {}: expected {}, got {}",
                    object.hash_hex(),
                    object.size,
                    bytes.len()
                ),
            });
        }
        let arc: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
        self.hot
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(object.hash, Arc::clone(&arc));
        Ok(arc)
    }

    /// Pin an object in the hot set (tier hint: [`Tier::Hot`]).
    pub fn pin_object(&self, object: &ObjectRef) -> Result<Tier> {
        let _ = self.get_object(object)?;
        self.pinned
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(object.hash);
        Ok(Tier::Hot)
    }

    /// Drop pin and remove from hot cache when not pinned.
    pub fn evict_object(&self, object: &ObjectRef) -> Result<Tier> {
        self.pinned
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&object.hash);
        self.hot
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&object.hash);
        Ok(Tier::Cold)
    }

    /// Current tier hint for an object.
    pub fn tier_of(&self, object: &ObjectRef) -> Tier {
        let pinned = self
            .pinned
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(&object.hash);
        if pinned {
            return Tier::Hot;
        }
        let hot = self
            .hot
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(&object.hash);
        if hot {
            Tier::Hot
        } else {
            Tier::Cold
        }
    }

    /// Write a vector payload into CAS.
    pub fn put_vector(&self, vector: &Vector) -> Result<ObjectRef> {
        self.put_chunk(ObjectKind::VectorPayload, &vector.to_le_bytes())
    }

    /// Read a vector payload from CAS.
    pub fn get_vector(&self, object: &ObjectRef) -> Result<Vector> {
        if object.kind != ObjectKind::VectorPayload {
            return Err(Error::ObjectCorrupt {
                message: "object kind is not VectorPayload".into(),
            });
        }
        let bytes = self.get_object(object)?;
        Vector::from_le_bytes(&bytes)
    }

    /// Chunk a logical file into CAS objects and return a manifest.
    pub fn put_file_chunked(
        &self,
        mut reader: impl Read,
        chunk_size: usize,
    ) -> Result<ChunkManifest> {
        let chunk_size = if chunk_size == 0 {
            DEFAULT_CHUNK_SIZE
        } else {
            chunk_size
        };
        let chunk_size_u32 = u32::try_from(chunk_size).map_err(|_| Error::ObjectCorrupt {
            message: "chunk_size exceeds u32".into(),
        })?;
        let mut chunks = Vec::new();
        let mut total_size = 0_u64;
        let mut buf = vec![0_u8; chunk_size];
        loop {
            let mut filled = 0;
            while filled < chunk_size {
                let n = reader.read(&mut buf[filled..])?;
                if n == 0 {
                    break;
                }
                filled += n;
            }
            if filled == 0 {
                break;
            }
            total_size += filled as u64;
            chunks.push(self.put_chunk(ObjectKind::Blob, &buf[..filled])?);
            if filled < chunk_size {
                break;
            }
        }
        if chunks.is_empty() {
            // Empty file: one zero-length chunk for a stable manifest.
            chunks.push(self.put_chunk(ObjectKind::Blob, &[])?);
        }
        Ok(ChunkManifest {
            chunk_size: chunk_size_u32,
            total_size,
            chunks,
        })
    }

    /// Read a byte range from a chunked file without assembling the whole file.
    pub fn read_file_range(
        &self,
        manifest: &ChunkManifest,
        offset: u64,
        len: usize,
    ) -> Result<Vec<u8>> {
        if len == 0 {
            return Ok(Vec::new());
        }
        if offset > manifest.total_size {
            return Err(Error::ObjectCorrupt {
                message: "read offset past end of file".into(),
            });
        }
        let end = offset
            .checked_add(len as u64)
            .ok_or(Error::ObjectCorrupt {
                message: "read range overflow".into(),
            })?
            .min(manifest.total_size);
        let need = (end - offset) as usize;
        let chunk_size = manifest.chunk_size as u64;
        if chunk_size == 0 {
            return Err(Error::ObjectCorrupt {
                message: "manifest chunk_size is zero".into(),
            });
        }
        let mut out = Vec::with_capacity(need);
        let mut cursor = offset;
        while cursor < end {
            let index = (cursor / chunk_size) as usize;
            let chunk = manifest.chunks.get(index).ok_or(Error::ObjectCorrupt {
                message: format!("missing chunk {index}"),
            })?;
            let data = self.get_object(chunk)?;
            let into_chunk = (cursor % chunk_size) as usize;
            if into_chunk >= data.len() {
                return Err(Error::ObjectCorrupt {
                    message: "chunk shorter than expected".into(),
                });
            }
            let take = ((end - cursor) as usize).min(data.len() - into_chunk);
            out.extend_from_slice(&data[into_chunk..into_chunk + take]);
            cursor += take as u64;
        }
        Ok(out)
    }
}
