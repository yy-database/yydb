//! Shared types for YYDB: errors, schema identity, values, and CAS refs.
//!
//! Applications should normally depend on the [`yydb`](https://docs.rs/yydb)
//! facade, which re-exports these types.

#![deny(missing_docs)]

use std::collections::BTreeMap;
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
    SchemaConflict {
        /// Expected schema version.
        expected: u32,
        /// Stored schema version.
        found: u32,
    },
    /// No scalar UDF is registered under this name.
    UdfNotFound {
        /// Missing UDF name.
        name: String,
    },
    /// UDF called with the wrong number of arguments.
    UdfArity {
        /// UDF name.
        name: String,
        /// Expected argument count.
        expected: usize,
        /// Supplied argument count.
        got: usize,
    },
    /// UDF body failed or rejected its arguments.
    Udf {
        /// UDF name.
        name: String,
        /// Failure message from the UDF host.
        message: String,
    },
    /// Schema document failed VOS validation (shared language contract).
    Schema {
        /// Validation failure message.
        message: String,
    },
    /// CAS object missing on disk / hot cache.
    ObjectNotFound {
        /// Lowercase hex digest of the missing object.
        hash_hex: String,
    },
    /// Chunked read or manifest is inconsistent.
    ObjectCorrupt {
        /// Corruption detail message.
        message: String,
    },
    /// Feature exists as a product surface but is not implemented yet.
    Unsupported(&'static str),
    /// Wire protocol / serve client failure.
    Protocol {
        /// Protocol error message.
        message: String,
    },
    /// Independent compare-and-set failed.
    CasConflict {
        /// Key that failed the compare.
        key: String,
    },
    /// Lease cannot be claimed under the given expectation.
    LeaseUnavailable {
        /// Lease key.
        key: String,
    },
    /// Lease token does not match the active claim.
    FencingMismatch {
        /// Lease key.
        key: String,
    },
    /// Lease has expired.
    LeaseExpired {
        /// Lease key.
        key: String,
    },
    /// Registered UDF version does not match the invocation.
    UdfVersionMismatch {
        /// UDF name.
        name: String,
        /// Expected registered version.
        expected: u32,
        /// Supplied invocation version.
        got: u32,
    },
    /// Namespace quota cannot admit another record.
    QuotaExceeded {
        /// Namespace that exceeded quota.
        namespace: String,
    },
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
            Self::CasConflict { key } => write!(f, "CAS conflict on key {key}"),
            Self::LeaseUnavailable { key } => write!(f, "lease unavailable on key {key}"),
            Self::FencingMismatch { key } => {
                write!(f, "fencing mismatch on key {key}")
            }
            Self::LeaseExpired { key } => write!(f, "lease expired on key {key}"),
            Self::UdfVersionMismatch {
                name,
                expected,
                got,
            } => write!(
                f,
                "UDF {name} version mismatch: expected {expected}, got {got}"
            ),
            Self::QuotaExceeded { namespace } => {
                write!(f, "quota exceeded for namespace {namespace}")
            }
        }
    }
}

/// Monotonic commit counter returned from fenced batch commits.
pub type CommitSequence = u64;

/// Handle proving an active lease claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseToken {
    /// Frontier or work item key under lease.
    pub key: String,
    /// Worker that holds the claim.
    pub worker_id: String,
    /// Monotonic fencing token assigned at claim time.
    pub fencing_token: u64,
    /// Wall-clock expiry in milliseconds since UNIX epoch.
    pub lease_until: u64,
}

/// Preconditions for [`yydb::Connection::claim_lease`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseExpectation {
    /// Key has no active claim or the prior claim expired.
    AbsentOrExpired,
    /// Key is in the queued state.
    Queued,
}

/// Per-key version counter returned from TTL writes.
pub type RecordVersion = u64;

/// Namespace byte and record limits with an eviction policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceQuota {
    /// Maximum total payload bytes under the namespace prefix.
    pub max_bytes: u64,
    /// Maximum record count under the namespace prefix.
    pub max_records: u64,
    /// How to choose victims when limits are exceeded.
    pub eviction_policy: EvictionPolicy,
}

/// Eviction strategy for namespace quota enforcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvictionPolicy {
    /// Evict the least recently touched record first.
    Lru,
    /// Evict the soonest-expiring TTL record first.
    TtlFirst,
}

/// Observed usage for a namespace prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NamespaceStats {
    /// Total bytes stored under the prefix.
    pub bytes_used: u64,
    /// Number of records under the prefix.
    pub records_used: u64,
}

/// Upper bound for a single eviction pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvictBudget {
    /// Maximum records to evict in one call.
    pub max_records: usize,
}

/// Severity for a [`DoctorIssue`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoctorSeverity {
    /// Non-fatal inconsistency or lag.
    Warn,
    /// Data or contract violation.
    Error,
}

/// One consistency finding from [`DoctorReport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorIssue {
    /// How severe the finding is.
    pub severity: DoctorSeverity,
    /// Stable diagnostic code such as `yydb.doctor.half_batch`.
    pub code: String,
    /// Human-readable detail.
    pub message: String,
    /// Optional record key hint.
    pub key_hint: Option<String>,
}

/// Per-namespace usage included in [`DoctorReport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorNamespaceStats {
    /// Namespace prefix.
    pub prefix: String,
    /// Records under the prefix.
    pub record_count: u64,
    /// Payload bytes under the prefix.
    pub bytes_used: u64,
    /// Referenced CAS payload bytes.
    pub object_bytes: u64,
    /// Configured byte quota, if any.
    pub quota_max_bytes: Option<u64>,
    /// TTL records past expiry but not yet evicted.
    pub ttl_expired_pending: u64,
}

/// Read-only consistency report from `Connection::doctor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorReport {
    /// Filesystem path when file-backed.
    pub file_path: Option<String>,
    /// Latest committed sequence number.
    pub last_commit_sequence: u64,
    /// Sequence folded into the main file at last checkpoint.
    pub last_checkpoint_sequence: u64,
    /// Commits not yet checkpointed.
    pub checkpoint_lag_records: u64,
    /// Uncheckpointed WAL bytes when in WAL mode.
    pub wal_bytes_uncheckpointed: u64,
    /// CAS objects without metadata references.
    pub orphan_object_count: u64,
    /// Namespace usage snapshots.
    pub namespaces: Vec<DoctorNamespaceStats>,
    /// Findings requiring attention.
    pub issues: Vec<DoctorIssue>,
}

/// Summary of CAS objects removed by orphan reclamation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReclaimReport {
    /// Objects deleted from the store.
    pub reclaimed_objects: u64,
}

/// Summary of keys removed by TTL or quota eviction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EvictReport {
    /// Records removed.
    pub evicted_records: u64,
    /// Payload bytes removed.
    pub evicted_bytes: u64,
}

/// How to release a lease after work finishes or fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseOutcome {
    /// Return the item to the queued state.
    Requeue,
    /// Schedule a retry at a future timestamp.
    RetryAt {
        /// Milliseconds since UNIX epoch.
        when: u64,
    },
    /// Mark the item failed with a reason string.
    Failed {
        /// Human-readable failure reason.
        reason: String,
    },
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
    /// Monotonic schema generation stored in the database.
    pub version: u32,
    /// Canonical VOS schema document text.
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
    /// SQL-style null.
    Null,
    /// Boolean scalar.
    Bool(bool),
    /// Signed 64-bit integer.
    I64(i64),
    /// Unsigned 64-bit integer.
    U64(u64),
    /// IEEE-754 double.
    F64(f64),
    /// Small inline bytes (allowed; prefer CAS when large).
    Bytes(Vec<u8>),
    /// UTF-8 text scalar.
    Text(String),
    /// UUID scalar.
    Uuid(uuid::Uuid),
    /// Logical vector (often materialized via [`ObjectRef`]).
    Vector(Vector),
    /// CAS handle under `<hash-prefix>/<chunk_hash>.blob`.
    Object(ObjectRef),
    /// Chunked logical file.
    File(ChunkManifest),
    /// Nested row embedded in query projection results (not persisted inline).
    Row(BTreeMap<String, Value>),
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
