//! YYDB public facade -- Rust embed `Connection` with **VOS as the only DDL and
//! query language**.
//!
//! ## Crate layering
//!
//! | Layer | Crate | Role |
//! |-------|-------|------|
//! | Contracts | `yydb-types` | `Error`, `Value`, CAS refs, UDF contracts |
//! | Physical | `yydb-formats` | page, B-tree, WAL, SHM and blob v0 layout |
//! | Durability | `yydb-storage` | file lifecycle, locking, snapshots, checkpoint, blob I/O |
//! | Runtime | `yydb-engine` | connection, transactions, catalog, leases, UDF sessions |
//! | **Facade** | **`yydb`** | module-wrapped re-exports, prelude, version |
//! | Wire | `yydb-protocol` | frames and bodies only |
//! | Remote | `yydb-client`, `yydb-server` | Rust remote client and serve loop (native only) |
//!
//! **Embedded host:** register native [UDFs](udf) on [`Connection`]. Remote Rust
//! hosts use the [`client`] module (or `yydb-client` directly). Language bindings
//! take every Rust capability from this facade. The native [`server`] module is
//! excluded on `wasm32`.
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
//!         _ => Err(yydb::Error::Udf { name: "double".into(), message: "expected i64".into() }),
//!     })?;
//!     conn.checkpoint()?;
//!     Ok(())
//! }
//! ```

#![deny(missing_docs)]

/// Shared data contracts (`Error`, `Result`, `Value`, CAS refs, ...).
pub mod types {
    pub use yydb_types::*;
}

/// YY wire frames, message tags and body codecs.
pub mod protocol {
    pub use yydb_protocol::*;
}

/// Rust remote client for `yydb serve`.
#[cfg(not(target_arch = "wasm32"))]
pub mod client {
    pub use yydb_client::*;
}

/// Binary serve loop for out-of-process clients.
#[cfg(not(target_arch = "wasm32"))]
pub mod server {
    pub use yydb_server::*;
}

/// UDF traits, registry and host adapters.
pub mod udf {
    pub use yydb_engine::udf::*;
}

/// VOS catalog publication and contract resolution.
pub mod catalog {
    pub use yydb_engine::catalog::*;
}

/// Durable connection handle and statement surface.
pub mod connection {
    pub use yydb_engine::connection::Connection;
}

/// Private SQL bridge — not part of the `@yydb/yydb` public author surface.
///
/// Internal crates (`sqlite`, integration tests) access SQL lowering and
/// execution through this module. Downstream authors should not depend on it.
#[doc(hidden)]
pub mod __sql_private {
    pub use yydb_query::{Dialect, SqlOutcome, SqlPlan, execute_sql, prepare_sql};
}

/// Batched write operations.
pub mod batch {
    pub use yydb_engine::batch::*;
}

/// Journal modes and `-wal` / `-shm` sidecar helpers.
pub mod journal {
    pub use yydb_engine::journal::*;
}

/// Unified `<hash-prefix>/<chunk_hash>.blob` CAS + hot/cold tiering.
pub mod objects {
    pub use yydb_engine::objects::*;
}

/// Query planning and DML operations (VOS frontend only on the public surface).
pub mod query {
    pub use yydb_query::{
        BoundStatement, BoundValue, CmpOp, ParameterDescriptor, Pred, PreparedStatement, ProjectExpr, ProjectField, QueryKind,
        QueryOp, QueryRow, SlotTy, SortKey, WriteAssignment, WriteField, WriteOp, WriteOutcome, bind_named, bind_ordered,
        execute, execute_prepared, execute_write, execute_write_prepared, execute_write_with_constraints, fingerprint, prepare,
        resolved_contract_from_document, try_insert_returning, upsert_row,
    };
}

/// Local expression programs used by UDF execution.
pub mod execution {
    pub use yydb_execution::*;
}

/// Upstream VOS language contracts (`ResolvedContract`, ...).
pub mod vos {
    pub use vos::*;
}

pub use batch::Batch;
pub use connection::Connection;
pub use journal::{JournalMode, OpenFlags};
pub use objects::ObjectStore;
pub use udf::{
    HostFunctionHandle, HostMicroDefinition, HostRuntimeAdapter, ScalarUdf, UdfError, UdfRegistry, UdfType, UdfValue,
};

/// Result alias for UDF host functions and registry operations.
pub type UdfResult<T = UdfValue> = yydb_udf::Result<T>;

pub use types::{
    ChunkManifest, CommitSequence, CompactReport, DEFAULT_CHUNK_SIZE, Date, Decimal, DoctorIssue, DoctorReport, DoctorSeverity,
    Error, EvictBudget, EvictReport, EvictionPolicy, HashAlgo, INLINE_BYTES_MAX, LeaseExpectation, LeaseToken, NamespaceQuota,
    NamespaceStats, ObjectKind, ObjectRef, ReclaimReport, RecordVersion, ReleaseOutcome, Result, SchemaVersion,
    TableUniqueConstraint, Tier, Time, Timestamp, UniqueConstraint, Value, Vector,
};

/// Common embed imports (`use yydb::prelude::*`).
pub mod prelude {
    pub use crate::{Batch, Connection, Error, JournalMode, ObjectRef, ObjectStore, OpenFlags, Result, SchemaVersion, Value};
}

/// Library version string (Cargo package version).
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
