//! YYDB public facade -- Rust embed `Connection` with **VOS as the only DDL and
//! query language**.
//!
//! ## Crate layering
//!
//! | Layer | Crate | Role |
//! |-------|-------|------|
//! | Bottom | `yydb-types` | `Error`, `Value`, CAS refs |
//! | Engine | `yydb-execution`, `yydb-udf`, `yydb-query` | Execution IR, UDF, query/DML |
//! | **Facade** | **`yydb`** | `Connection`, `wire`, re-exports |
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
//! runtime hot/cold tiering — not a `.yydb-vec` sidecar.
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

/// Shared types (`Error`, `Result`, `Value`, …).
pub mod types {
    pub use yydb_types::*;
}

/// Journal modes and `-wal` / `-shm` sidecar helpers.
pub mod journal;

/// Unified `<hash-prefix>/<chunk_hash>.blob` CAS + hot/cold tiering.
pub mod objects;

/// Shared VOS schema contract (`vos` git @ `dev`).
pub mod schema;

/// UDF traits (`ScalarUdf`) and registry types (`UdfRegistry`, ...).
pub mod udf;

pub mod query {
    //! Phase 1 VOS query and DML -- re-exported from the [`yydb_query`] engine crate.
    //!
    //! Prefer [`crate::Connection::query`] and [`crate::Connection::execute`] for embed hosts. Call
    //! [`execute`](yydb_query::execute) directly only when driving a custom record map.

    pub use yydb_query::*;
}

/// Common embed imports (`use yydb::prelude::*`).
pub mod prelude {
    pub use crate::{
        Batch, Connection, Error, JournalMode, ObjectRef, ObjectStore, OpenFlags, QueryRow, Result,
        SchemaVersion, Value,
    };
}

mod doctor;
mod file_lock;
mod format_v0;
mod lease;
mod refs;
mod ttl;
mod udf_bridge;

/// YY wire protocol (`YYDB`|`YYDS` + version digits `0000`…).
pub mod wire;

pub use journal::{JournalMode, OpenFlags};
pub use objects::ObjectStore;
pub use query::QueryRow;
pub use udf::ScalarUdf;

pub use yydb_execution as execution;
pub use yydb_query;
pub use yydb_types;
pub use yydb_udf;

pub use udf::{
    HostFunctionHandle, HostMicroDefinition, HostRuntimeAdapter, UdfError, UdfRegistry, UdfType,
    UdfValue,
};
/// Result alias for UDF host functions and registry operations.
pub type UdfResult<T = UdfValue> = yydb_udf::Result<T>;
pub use yydb_types::{
    ChunkManifest, CommitSequence, DoctorIssue, DoctorReport, DoctorSeverity, Error, EvictBudget,
    EvictReport, EvictionPolicy, HashAlgo, LeaseExpectation, LeaseToken, NamespaceQuota,
    NamespaceStats, ObjectKind, ObjectRef, ReclaimReport, RecordVersion, ReleaseOutcome, Result,
    SchemaVersion, Tier, Value, Vector, DEFAULT_CHUNK_SIZE, INLINE_BYTES_MAX,
};

/// VOS schema language facade (`git+https://github.com/voml/vos-language.git?branch=dev`).
pub use vos;

mod batch;
pub mod connection;

pub use batch::Batch;
pub use connection::Connection;

/// Library version string (Cargo package version).
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
