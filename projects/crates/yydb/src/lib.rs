//! YYDB public facade -- Rust embed `Connection` with **VOS as the only DDL and
//! query language**.
//!
//! ## Crate layering
//!
//! | Layer | Crate | Role |
//! |-------|-------|------|
//! | Bottom | `yydb-types` | `Error`, `Value`, CAS refs |
//! | Engine | `yydb-store`, `yydb-format`, `yydb-execution`, `yydb-udf`, `yydb-query` | Storage, formats, execution IR, UDF, query |
//! | **Facade** | **`yydb`** | Re-exports, prelude, version |
//! | Protocol | `yydb-protocol` | Wire frames and bodies (no database access) |
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
//! use the unified [`objects`] CAS (`<hash-prefix>/<chunk_hash>.blob`) with optional
//! runtime hot/cold tiering -- not a `.yydb-vec` sidecar.
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

/// Shared types (`Error`, `Result`, `Value`, ...).
pub mod types {
    pub use yydb_types::*;
}

/// Journal modes and `-wal` / `-shm` sidecar helpers.
pub use yydb_store::journal;

/// Unified `<hash-prefix>/<chunk_hash>.blob` CAS + hot/cold tiering.
pub use yydb_store::objects;

/// Shared VOS schema contract.
pub use yydb_store::catalog as schema;

/// UDF traits (`ScalarUdf`) and registry types (`UdfRegistry`, ...).
pub use yydb_store::udf;

/// Common embed imports (`use yydb::prelude::*`).
pub mod prelude {
    pub use crate::{
        Batch, Connection, Error, JournalMode, ObjectRef, ObjectStore, OpenFlags, Result,
        SchemaVersion, Value,
    };
}

/// Batched write operations.
pub use yydb_store::batch;

/// Durable connection handle and statement surface.
pub use yydb_store::connection;

/// YY wire protocol (`YYDB`|`YYDS` + version digits `0000`...).
pub use yydb_protocol as wire;

/// Query planning and DML operations.
pub use yydb_query as query;

pub use yydb_store::{
    Batch, Connection, HostFunctionHandle, HostMicroDefinition, HostRuntimeAdapter, JournalMode,
    ObjectStore, OpenFlags, ScalarUdf, UdfError, UdfRegistry, UdfType, UdfValue,
};
/// Result alias for UDF host functions and registry operations.
pub type UdfResult<T = UdfValue> = yydb_udf::Result<T>;

pub use yydb_execution as execution;
pub use yydb_types;
pub use yydb_types::{
    ChunkManifest, CommitSequence, CompactReport, DoctorIssue, DoctorReport, DoctorSeverity, Error,
    EvictBudget, EvictReport, EvictionPolicy, HashAlgo, LeaseExpectation, LeaseToken,
    NamespaceQuota, NamespaceStats, ObjectKind, ObjectRef, ReclaimReport, RecordVersion,
    ReleaseOutcome, Result, SchemaVersion, Tier, Value, Vector, DEFAULT_CHUNK_SIZE,
    INLINE_BYTES_MAX,
};
pub use yydb_udf;

/// VOS schema language facade.
pub use vos;

/// Library version string (Cargo package version).
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
