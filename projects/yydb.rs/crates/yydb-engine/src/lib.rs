//! YYDB database runtime: connection state, VOS catalog publication, query
//! invocation, CAS coordination, lease/TTL, backup and the UDF session bridge.
//!
//! This crate owns runtime behaviour, and the `yydb` facade re-exports it
//! without growing implementations of its own.

#![deny(missing_docs)]

/// Batched write operations.
pub mod batch;
/// `bytes` placement at the inline boundary and object-reference row decoding.
pub mod bytes;
/// VOS catalog publication and contract resolution.
pub mod catalog;
/// Durable connection handle and statement surface.
pub mod connection;
/// Journal modes and `-wal` / `-shm` sidecar helpers.
pub mod journal;
/// Unified `<hash-prefix>/<chunk_hash>.blob` CAS + hot/cold tiering.
pub mod objects;
/// UDF traits (`ScalarUdf`) and registry types (`UdfRegistry`, ...).
pub mod udf;

mod kv;
mod storage;

pub use batch::Batch;
pub use bytes::{ObjectStoreResolver, YydbBytesPolicy};
pub use connection::Connection;
pub use journal::{JournalMode, OpenFlags};
pub use objects::ObjectStore;
pub use udf::ScalarUdf;

pub use udf::{HostFunctionHandle, HostMicroDefinition, HostRuntimeAdapter, UdfError, UdfRegistry, UdfType, UdfValue};
/// Result alias for UDF host functions and registry operations.
pub type UdfResult<T = UdfValue> = yydb_udf::Result<T>;

pub use yydb_execution as execution;
pub use yydb_query::QueryRow;
pub use yydb_types::{
    self, ChunkManifest, CommitSequence, CompactReport, DEFAULT_CHUNK_SIZE, DoctorIssue, DoctorReport, DoctorSeverity, Error,
    EvictBudget, EvictReport, EvictionPolicy, FileDescriptor, FileManifestRef, HashAlgo, INLINE_BYTES_MAX, LeaseExpectation,
    LeaseToken, NamespaceQuota, NamespaceStats, ObjectKind, ObjectRef, ReclaimReport, RecordVersion, ReleaseOutcome, Result,
    SchemaVersion, Tier, Value, Vector,
};
pub use yydb_udf;

/// Storage-engine version string (Cargo package version).
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
