//! Shared types for YYDB: errors, schema identity, values, and CAS refs.
//!
//! Applications should normally depend on the [`yydb`](https://docs.rs/yydb)
//! facade, which re-exports these types.

use std::{error, fmt, io, result, sync::Arc};

/// Soft recommendation: prefer CAS above this size instead of row-inline bytes.
pub const INLINE_BYTES_MAX: usize = 4 * 1024;

/// Default chunk size for chunked file writes (1 MiB).
pub const DEFAULT_CHUNK_SIZE: usize = 1024 * 1024;

/// Convenient alias used across YYDB APIs.
pub type Result<T = ()> = result::Result<T, Error>;

/// Errors produced by YYDB open / read / write / UDF / object paths.
#[derive(Debug)]
pub enum Error {
    /// Underlying filesystem or I/O failure.
    Io(io::Error),
    /// File contents do not match the expected layout.
    Corrupt(&'static str),
    /// Stored schema version does not match the caller’s expectation.
    SchemaConflict { expected: u32, found: u32 },
    /// No scalar UDF is registered under this name.
    UdfNotFound { name: String },
    /// UDF called with the wrong number of arguments.
    UdfArity {
        name: String,
        expected: usize,
        got: usize,
    },
    /// UDF body failed or rejected its arguments.
    Udf { name: String, message: String },
    /// Schema document failed VOS validation (shared language contract).
    Schema { message: String },
    /// CAS object missing on disk / hot cache.
    ObjectNotFound { hash_hex: String },
    /// Chunked read or manifest is inconsistent.
    ObjectCorrupt { message: String },
    /// Feature exists as a product surface but is not implemented yet.
    Unsupported(&'static str),
    /// Wire protocol / serve client failure.
    Protocol { message: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::Corrupt(message) => write!(f, "corrupt YYDB file: {message}"),
            Self::SchemaConflict { expected, found } => {
                write!(
                    f,
                    "schema version conflict: expected {expected}, found {found}"
                )
            }
            Self::UdfNotFound { name } => write!(f, "UDF not found: {name}"),
            Self::UdfArity {
                name,
                expected,
                got,
            } => write!(
                f,
                "UDF {name} arity mismatch: expected {expected} args, got {got}"
            ),
            Self::Udf { name, message } => write!(f, "UDF {name}: {message}"),
            Self::Schema { message } => write!(f, "VOS schema: {message}"),
            Self::ObjectNotFound { hash_hex } => {
                write!(f, "object not found: {hash_hex}")
            }
            Self::ObjectCorrupt { message } => write!(f, "object corrupt: {message}"),
            Self::Unsupported(feature) => write!(f, "unsupported: {feature}"),
            Self::Protocol { message } => write!(f, "protocol: {message}"),
        }
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Database-owned VOS schema document and version (database truth).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaVersion {
    pub version: u32,
    pub document: String,
}

/// Content digest algorithm recorded on [`ObjectRef`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HashAlgo {
    /// BLAKE3-256 (default).
    Blake3,
}

impl HashAlgo {
    /// Stable name for diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blake3 => "blake3",
        }
    }
}

/// What a CAS object represents (same path layout for all kinds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ObjectKind {
    /// Opaque blob / file chunk.
    Blob,
    /// Encoded vector payload.
    VectorPayload,
    /// Rebuildable ANN graph segment.
    AnnSegment,
}

/// Runtime residency hint (cache policy; not a second on-disk format).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tier {
    /// Prefer keeping the object in the process hot set.
    Hot,
    /// On-disk CAS only until faulted in.
    Cold,
}

/// Content-addressed object handle (rows store this, not raw payloads).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ObjectRef {
    /// Digest algorithm.
    pub algo: HashAlgo,
    /// Raw digest bytes (32 for BLAKE3).
    pub hash: [u8; 32],
    /// Payload size in bytes.
    pub size: u64,
    /// Semantic kind.
    pub kind: ObjectKind,
}

impl ObjectRef {
    /// Lowercase hex encoding of [`Self::hash`].
    pub fn hash_hex(&self) -> String {
        hex::encode(self.hash)
    }

    /// `hash-2` directory segment (first two hex characters).
    pub fn hash_prefix2(&self) -> String {
        self.hash_hex().chars().take(2).collect()
    }
}

/// Ordered chunk list for a logical file (or single-chunk small file).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkManifest {
    /// Nominal chunk size used when writing (last chunk may be shorter).
    pub chunk_size: u32,
    /// Total logical byte length.
    pub total_size: u64,
    /// Chunks in order; each points at `objects/hash-2/*.bytes`.
    pub chunks: Vec<ObjectRef>,
}

/// Fixed-dimension float vector (logical value; prefer CAS for persistence).
#[derive(Debug, Clone, PartialEq)]
pub struct Vector {
    /// Dimension (must match `data.len()`).
    pub dim: u32,
    /// Contiguous `f32` components.
    pub data: Arc<[f32]>,
}

impl Vector {
    /// Create a vector; `data.len()` must fit in `u32` and becomes `dim`.
    pub fn new(data: impl Into<Arc<[f32]>>) -> Result<Self> {
        let data = data.into();
        let dim = u32::try_from(data.len()).map_err(|_| Error::ObjectCorrupt {
            message: "vector dimension exceeds u32".into(),
        })?;
        Ok(Self { dim, data })
    }

    /// Encode as little-endian `f32` bytes for CAS storage.
    pub fn to_le_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.data.len() * 4);
        for value in self.data.iter() {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }

    /// Decode little-endian `f32` bytes.
    pub fn from_le_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() % 4 != 0 {
            return Err(Error::ObjectCorrupt {
                message: "vector payload length not multiple of 4".into(),
            });
        }
        let mut data = Vec::with_capacity(bytes.len() / 4);
        for chunk in bytes.chunks_exact(4) {
            data.push(f32::from_le_bytes(chunk.try_into().unwrap()));
        }
        Self::new(data)
    }
}

/// Minimal typed value set for YYDB.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Value {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    /// Small inline bytes (allowed; prefer CAS when large).
    Bytes(Vec<u8>),
    Text(String),
    Uuid(uuid::Uuid),
    /// Logical vector (often materialized via [`ObjectRef`]).
    Vector(Vector),
    /// CAS handle under `objects/hash-2/*.bytes`.
    Object(ObjectRef),
    /// Chunked logical file.
    File(ChunkManifest),
}

mod hex {
    pub fn encode(bytes: [u8; 32]) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(64);
        for byte in bytes {
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0xf) as usize] as char);
        }
        out
    }
}
