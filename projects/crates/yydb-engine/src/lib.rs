//! YYDB database runtime: connection state, VOS catalog publication, query
//! invocation, CAS coordination, lease/TTL, backup and the UDF session bridge.
//!
//! This crate owns runtime behaviour, and the `yydb` facade re-exports it
//! without growing implementations of its own.

#![deny(missing_docs)]

/// Batched write operations.
pub mod batch;
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
/// Historic module alias kept for `yydb::schema` compatibility.
pub use catalog as schema;
pub use connection::Connection;
pub use journal::{JournalMode, OpenFlags};
pub use objects::ObjectStore;
pub use udf::ScalarUdf;

pub use udf::{
    HostFunctionHandle, HostMicroDefinition, HostRuntimeAdapter, UdfError, UdfRegistry, UdfType,
    UdfValue,
};
/// Result alias for UDF host functions and registry operations.
pub type UdfResult<T = UdfValue> = yydb_udf::Result<T>;

pub use yydb_execution as execution;
pub use yydb_query::QueryRow;
pub use yydb_types;
pub use yydb_types::{
    ChunkManifest, CommitSequence, CompactReport, DoctorIssue, DoctorReport, DoctorSeverity, Error,
    EvictBudget, EvictReport, EvictionPolicy, HashAlgo, LeaseExpectation, LeaseToken,
    NamespaceQuota, NamespaceStats, ObjectKind, ObjectRef, ReclaimReport, RecordVersion,
    ReleaseOutcome, Result, SchemaVersion, Tier, Value, Vector, DEFAULT_CHUNK_SIZE,
    INLINE_BYTES_MAX,
};
pub use yydb_udf;

/// Storage-engine version string (Cargo package version).
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
