//! Unified content-addressed store: `<store>/<hash-prefix>/<chunk_hash>.blob`.
//!
//! Large logical files are **chunked** manifests; vectors and ANN segments use
//! the same path layout. Hot/cold tiering is an in-process cache over this CAS.
//!
//! Production layout: `.yydx` uses `<app-name>-objects/` with `YYBB` segments.
//! `.yydb` single-file mode must not rely on persistent sidecars.

use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone)]
enum ObjectStoreRoot {
    /// Process-local CAS only. No filesystem writes.
    Ephemeral,
    /// Durable CAS rooted at `<store>/<hash-prefix>/<chunk_hash>.blob`.
    OnDisk(BlobStore),
}

use sha2::{Digest, Sha256};
use yydb_storage::{BlobChunkRef, BlobStore, blob_chunk_hash};
use yydb_types::{
    ChunkManifest, DEFAULT_CHUNK_SIZE, Error, FileDescriptor, FileManifestRef, HashAlgo, INLINE_BYTES_MAX, ObjectKind,
    ObjectRef, Result, Tier, Vector,
};

/// Soft inline size guidance (re-exported for callers).
pub use yydb_types::INLINE_BYTES_MAX as INLINE_BYTES_SOFT_MAX;

/// Content-addressed object store rooted beside a `.yydb` file.
pub struct ObjectStore {
    root: ObjectStoreRoot,
    hot: Mutex<HashMap<[u8; 32], Arc<[u8]>>>,
    /// Process-local cold tier when no on-disk CAS is available.
    cold: Mutex<HashMap<[u8; 32], Arc<[u8]>>>,
    /// In-process object index for ephemeral stores (`scan_orphans` / doctor).
    catalog: Mutex<HashMap<[u8; 32], ObjectRef>>,
    pinned: Mutex<HashSet<[u8; 32]>>,
}

impl ObjectStore {
    /// Process-local CAS for `Connection::open_in_memory` (no filesystem side effects).
    pub fn open_ephemeral() -> Self {
        Self {
            root: ObjectStoreRoot::Ephemeral,
            hot: Mutex::new(HashMap::new()),
            cold: Mutex::new(HashMap::new()),
            catalog: Mutex::new(HashMap::new()),
            pinned: Mutex::new(HashSet::new()),
        }
    }

    /// On-disk CAS under `root` (integration tests that need filesystem objects).
    pub fn open_in_memory_root(root: PathBuf) -> Result<Self> {
        Self::open_on_disk_root(root)
    }

    /// On-disk blob root for `.yydx` layouts (`<app-name>-objects/`).
    pub fn open_yydx_blob_root(root: PathBuf) -> Result<Self> {
        Self::open_on_disk_root(root)
    }

    /// Resolve `<app-name>-objects/` beside `app.yydx`.
    pub fn yydx_objects_root(main_path: &Path) -> PathBuf {
        let parent = main_path.parent().unwrap_or_else(|| Path::new("."));
        let stem = main_path.file_stem().and_then(|name| name.to_str()).unwrap_or("app");
        parent.join(format!("{stem}-objects"))
    }

    fn open_on_disk_root(root: PathBuf) -> Result<Self> {
        Ok(Self {
            root: ObjectStoreRoot::OnDisk(BlobStore::open(root)?),
            hot: Mutex::new(HashMap::new()),
            cold: Mutex::new(HashMap::new()),
            catalog: Mutex::new(HashMap::new()),
            pinned: Mutex::new(HashSet::new()),
        })
    }

    /// Absolute store root when on disk; synthetic path for ephemeral stores.
    pub fn root(&self) -> &Path {
        match &self.root {
            ObjectStoreRoot::Ephemeral => Path::new("/yydb-ephemeral"),
            ObjectStoreRoot::OnDisk(store) => store.root(),
        }
    }

    pub(crate) fn persist_to_disk(&self) -> bool {
        matches!(self.root, ObjectStoreRoot::OnDisk(_))
    }

    /// Relative blob store root (`<app-name>-objects/`).
    pub fn cas_root(&self) -> PathBuf {
        self.root().to_path_buf()
    }

    /// Path for a chunk digest: `<store>/<hash-prefix>/<chunk_hash>.blob`.
    pub fn path_for_hash(&self, hash_hex: &str) -> PathBuf {
        match &self.root {
            ObjectStoreRoot::Ephemeral => {
                let prefix: String = hash_hex.chars().take(2).collect();
                self.cas_root().join(prefix).join(format!("{hash_hex}.blob"))
            }
            ObjectStoreRoot::OnDisk(store) => store.path_for_hash(hash_hex),
        }
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
        let hash_bytes = blob_chunk_hash(bytes);
        let object = ObjectRef { algo: HashAlgo::Blake3, hash: hash_bytes, size: bytes.len() as u64, kind };
        if let ObjectStoreRoot::OnDisk(store) = &self.root {
            store.put(&object.hash_hex(), bytes)?;
        }
        self.hot.lock().unwrap_or_else(|p| p.into_inner()).insert(hash_bytes, Arc::<[u8]>::from(bytes.to_vec()));
        if !self.persist_to_disk() {
            self.catalog.lock().unwrap_or_else(|p| p.into_inner()).insert(hash_bytes, object.clone());
        }
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
        if let ObjectStoreRoot::OnDisk(store) = &self.root {
            let payload = store.read(&object.hash_hex(), object.size)?;
            let arc: Arc<[u8]> = Arc::from(payload.into_boxed_slice());
            self.hot.lock().unwrap_or_else(|p| p.into_inner()).insert(object.hash, Arc::clone(&arc));
            return Ok(arc);
        }
        let cold = self.cold.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(bytes) = cold.get(&object.hash) {
            return Ok(Arc::clone(bytes));
        }
        Err(Error::ObjectNotFound { hash_hex: object.hash_hex() })
    }

    /// Verify one referenced object is stored and reads back as the exact content
    /// its hash identifies.
    ///
    /// Metadata reference closure only keeps hashes, so the stored segment supplies
    /// its own declared length while the identity must still match the hash the
    /// committed record points at. The hot cache is deliberately bypassed: this is
    /// the strict gate a restored backup must clear, not a tier hint.
    pub(crate) fn verify_object_hash(&self, hash: &[u8; 32]) -> Result<()> {
        let hash_hex = hex_lower(hash);
        if let ObjectStoreRoot::OnDisk(store) = &self.root {
            let chunk = store.chunk_ref(&hash_hex)?;
            if chunk.chunk_hash != *hash {
                return Err(Error::ObjectCorrupt {
                    message: format!("blob {hash_hex} does not carry the hash its reference declares"),
                });
            }
            store.read(&hash_hex, u64::from(chunk.chunk_len))?;
            return Ok(());
        }
        let object = {
            let catalog = self.catalog.lock().unwrap_or_else(|p| p.into_inner());
            catalog.get(hash).cloned()
        };
        let Some(object) = object
        else {
            return Err(Error::ObjectNotFound { hash_hex });
        };
        drop(self.get_object(&object)?);
        Ok(())
    }

    /// Pin an object in the hot set (tier hint: [`Tier::Hot`]).
    pub fn pin_object(&self, object: &ObjectRef) -> Result<Tier> {
        let _ = self.get_object(object)?;
        self.pinned.lock().unwrap_or_else(|p| p.into_inner()).insert(object.hash);
        Ok(Tier::Hot)
    }

    /// Drop pin and remove from hot cache when not pinned.
    pub fn evict_object(&self, object: &ObjectRef) -> Result<Tier> {
        self.pinned.lock().unwrap_or_else(|p| p.into_inner()).remove(&object.hash);
        let mut hot = self.hot.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(bytes) = hot.remove(&object.hash) {
            if !self.persist_to_disk() {
                self.cold.lock().unwrap_or_else(|p| p.into_inner()).insert(object.hash, bytes);
            }
        }
        Ok(Tier::Cold)
    }

    /// Current tier hint for an object.
    pub fn tier_of(&self, object: &ObjectRef) -> Tier {
        let pinned = self.is_pinned(object);
        if pinned {
            return Tier::Hot;
        }
        let hot = self.hot.lock().unwrap_or_else(|p| p.into_inner()).contains_key(&object.hash);
        if hot { Tier::Hot } else { Tier::Cold }
    }

    /// Whether an object is pinned against GC reclaim.
    pub fn is_pinned(&self, object: &ObjectRef) -> bool {
        self.pinned.lock().unwrap_or_else(|p| p.into_inner()).contains(&object.hash)
    }

    /// Whether chunk bytes are still present in this store.
    pub(crate) fn is_stored(&self, object: &ObjectRef) -> bool {
        if self.persist_to_disk() {
            return self.path_for(object).exists();
        }
        if self.catalog.lock().unwrap_or_else(|p| p.into_inner()).contains_key(&object.hash) {
            return true;
        }
        if self.hot.lock().unwrap_or_else(|p| p.into_inner()).contains_key(&object.hash) {
            return true;
        }
        self.cold.lock().unwrap_or_else(|p| p.into_inner()).contains_key(&object.hash)
    }

    /// Write a vector payload into CAS.
    pub fn put_vector(&self, vector: &Vector) -> Result<ObjectRef> {
        self.put_chunk(ObjectKind::VectorPayload, &vector.to_le_bytes())
    }

    /// Read a vector payload from CAS.
    pub fn get_vector(&self, object: &ObjectRef) -> Result<Vector> {
        if object.kind != ObjectKind::VectorPayload {
            return Err(Error::ObjectCorrupt { message: "object kind is not VectorPayload".into() });
        }
        let bytes = self.get_object(object)?;
        Vector::from_le_bytes(&bytes)
    }

    /// Chunk a logical file into CAS objects and return a manifest.
    pub fn put_file_chunked(&self, mut reader: impl Read, chunk_size: usize) -> Result<ChunkManifest> {
        let chunk_size = if chunk_size == 0 { DEFAULT_CHUNK_SIZE } else { chunk_size };
        let chunk_size_u32 =
            u32::try_from(chunk_size).map_err(|_| Error::ObjectCorrupt { message: "chunk_size exceeds u32".into() })?;
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
        Ok(ChunkManifest { chunk_size: chunk_size_u32, total_size, chunks })
    }

    /// Publish a logical file as chunks plus an immutable manifest.
    ///
    /// Chunks are written first, then the manifest object, and the returned
    /// descriptor only refers to both: the row that stores it commits afterwards,
    /// so a rollback leaves unreferenced objects for `reclaim_orphans` instead of
    /// deleting content another row may already share. The SHA-256 digest covers
    /// the complete octets, so a rename or media-type change never alters the
    /// content identity.
    pub fn put_file(
        &self,
        reader: impl Read,
        chunk_size: usize,
        name: Option<String>,
        media_type: Option<String>,
    ) -> Result<FileDescriptor> {
        let mut hashing = HashingReader { inner: reader, hasher: Sha256::new() };
        let manifest = self.put_file_chunked(&mut hashing, chunk_size)?;
        let digest = hex_lower(&hashing.hasher.finalize());
        let payload = manifest.to_payload_bytes()?;
        let manifest_object = self.put_chunk(ObjectKind::FileManifest, &payload)?;
        Ok(FileDescriptor {
            digest,
            octets: manifest.total_size,
            name,
            media_type,
            manifest: FileManifestRef { hash: manifest_object.hash, size: manifest_object.size },
        })
    }

    /// Load and verify the manifest a `file` descriptor points at.
    ///
    /// The manifest object is content-addressed, so both its stored length and
    /// its BLAKE3 digest must match the reference before its chunk list is
    /// trusted. A manifest whose `total_size` disagrees with the descriptor is
    /// corruption, not a short read.
    pub fn load_file_manifest(&self, descriptor: &FileDescriptor) -> Result<ChunkManifest> {
        let object = descriptor.manifest_object();
        let payload = self.get_object(&object)?;
        if payload.len() as u64 != object.size {
            return Err(Error::ObjectCorrupt { message: "file manifest length does not match its reference".into() });
        }
        if blob_chunk_hash(payload.as_ref()) != object.hash {
            return Err(Error::ObjectCorrupt { message: "file manifest hash does not match its reference".into() });
        }
        let manifest = ChunkManifest::from_payload_bytes(&payload)?;
        if manifest.total_size != descriptor.octets {
            return Err(Error::ObjectCorrupt {
                message: format!(
                    "file manifest total size {} disagrees with descriptor octets {}",
                    manifest.total_size, descriptor.octets
                ),
            });
        }
        Ok(manifest)
    }

    /// Read one resolved VOS `FileReadWindow` of a `file` value.
    ///
    /// The window follows VOS semantics: an offset at or past end of file yields
    /// an empty window, and a request starting before end of file is truncated
    /// to the remaining octets, so a read past the end is a short read instead
    /// of an error. Every chunk the window touches is verified against the
    /// manifest hash and length before its octets are returned.
    pub fn read_file_window(&self, descriptor: &FileDescriptor, offset: u64, length: u64) -> Result<Vec<u8>> {
        let manifest = self.load_file_manifest(descriptor)?;
        let (offset, length) = descriptor.resolve_window(offset, length);
        if length == 0 {
            return Ok(Vec::new());
        }
        self.read_chunks_window(&manifest, offset, length)
    }

    /// Read the complete octets of a `file` value and verify its content digest.
    ///
    /// The SHA-256 digest covers every octet, so a descriptor whose digest was
    /// rewritten or whose chunks were reordered fails instead of returning
    /// plausible-looking bytes.
    pub fn read_file(&self, descriptor: &FileDescriptor) -> Result<Vec<u8>> {
        let octets = self.read_file_window(descriptor, 0, descriptor.octets)?;
        if octets.len() as u64 != descriptor.octets {
            return Err(Error::ObjectCorrupt {
                message: format!("file read produced {} octets, expected {}", octets.len(), descriptor.octets),
            });
        }
        if hex_lower(&Sha256::digest(&octets)) != descriptor.digest {
            return Err(Error::ObjectCorrupt { message: "file digest does not match the descriptor".into() });
        }
        Ok(octets)
    }

    /// Read a byte range from a chunked file without assembling the whole file.
    ///
    /// A range that starts at or past the manifest total is an empty range, not
    /// an error. Each addressed chunk is hash- and length-checked first.
    pub fn read_file_range(&self, manifest: &ChunkManifest, offset: u64, len: usize) -> Result<Vec<u8>> {
        if len == 0 || offset >= manifest.total_size {
            return Ok(Vec::new());
        }
        let length = (len as u64).min(manifest.total_size - offset);
        self.read_chunks_window(manifest, offset, length)
    }

    /// Copy one verified window out of the manifest's chunks.
    fn read_chunks_window(&self, manifest: &ChunkManifest, offset: u64, length: u64) -> Result<Vec<u8>> {
        let chunk_size = u64::from(manifest.chunk_size);
        if chunk_size == 0 {
            return Err(Error::ObjectCorrupt { message: "manifest chunk_size is zero".into() });
        }
        let end = offset.checked_add(length).ok_or(Error::ObjectCorrupt { message: "read range overflow".into() })?;
        let mut out = Vec::with_capacity(length as usize);
        let mut cursor = offset;
        while cursor < end {
            let index = (cursor / chunk_size) as usize;
            let chunk = manifest.chunks.get(index).ok_or_else(|| Error::ObjectCorrupt {
                message: format!("missing chunk {index} for read at offset {cursor}"),
            })?;
            let data = self.verify_chunk(chunk)?;
            let into_chunk = (cursor % chunk_size) as usize;
            if into_chunk >= data.len() {
                return Err(Error::ObjectCorrupt { message: format!("chunk {index} is shorter than the manifest declares") });
            }
            let take = ((end - cursor) as usize).min(data.len() - into_chunk);
            out.extend_from_slice(&data[into_chunk..into_chunk + take]);
            cursor += take as u64;
        }
        Ok(out)
    }

    /// Load one manifest chunk and verify its hash and declared length.
    fn verify_chunk(&self, chunk: &ObjectRef) -> Result<Arc<[u8]>> {
        if chunk.kind != ObjectKind::Blob {
            return Err(Error::ObjectCorrupt { message: "file manifest chunk is not a blob object".into() });
        }
        let data = self.get_object(chunk)?;
        if data.len() as u64 != chunk.size {
            return Err(Error::ObjectCorrupt {
                message: format!("chunk {} length does not match the manifest", chunk.hash_hex()),
            });
        }
        if blob_chunk_hash(data.as_ref()) != chunk.hash {
            return Err(Error::ObjectCorrupt {
                message: format!("chunk {} hash does not match its stored payload", chunk.hash_hex()),
            });
        }
        Ok(data)
    }

    /// Enumerate every `.blob` segment under the store root.
    pub fn list_objects(&self) -> Result<Vec<ObjectRef>> {
        if !self.persist_to_disk() {
            return Ok(self.catalog.lock().unwrap_or_else(|p| p.into_inner()).values().cloned().collect());
        }
        let ObjectStoreRoot::OnDisk(store) = &self.root
        else {
            return Ok(Vec::new());
        };
        store
            .list()?
            .into_iter()
            .map(|(hash_hex, size)| {
                Ok(ObjectRef { algo: HashAlgo::Blake3, hash: decode_hash_hex(&hash_hex)?, size, kind: ObjectKind::Blob })
            })
            .collect()
    }

    /// Read the `YYBB` header fields for a chunk already stored in this CAS.
    pub(crate) fn chunk_ref(&self, hash: &[u8; 32]) -> Result<BlobChunkRef> {
        let hash_hex = ObjectRef { algo: HashAlgo::Blake3, hash: *hash, size: 0, kind: ObjectKind::Blob }.hash_hex();
        match &self.root {
            ObjectStoreRoot::OnDisk(store) => store.chunk_ref(&hash_hex),
            ObjectStoreRoot::Ephemeral => Err(Error::ObjectNotFound { hash_hex }),
        }
    }

    /// Delete a CAS object when it is no longer referenced.
    pub fn remove_object(&self, object: &ObjectRef) -> Result<()> {
        if let ObjectStoreRoot::OnDisk(store) = &self.root {
            store.remove(&object.hash_hex())?;
        }
        self.hot.lock().unwrap_or_else(|p| p.into_inner()).remove(&object.hash);
        self.cold.lock().unwrap_or_else(|p| p.into_inner()).remove(&object.hash);
        self.catalog.lock().unwrap_or_else(|p| p.into_inner()).remove(&object.hash);
        self.pinned.lock().unwrap_or_else(|p| p.into_inner()).remove(&object.hash);
        Ok(())
    }
}

/// Reader adapter that hashes every octet it hands to the chunker.
struct HashingReader<R> {
    inner: R,
    hasher: Sha256,
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.hasher.update(&buf[..read]);
        Ok(read)
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

fn decode_hash_hex(hash_hex: &str) -> Result<[u8; 32]> {
    let mut hash = [0_u8; 32];
    for (index, chunk) in hash_hex.as_bytes().chunks(2).enumerate() {
        if chunk.len() != 2 {
            return Err(Error::ObjectCorrupt { message: "hash hex has odd length".into() });
        }
        let byte = u8::from_str_radix(&String::from_utf8_lossy(chunk), 16)
            .map_err(|_| Error::ObjectCorrupt { message: "hash hex is not valid".into() })?;
        hash[index] = byte;
    }
    Ok(hash)
}

pub(crate) mod references;
