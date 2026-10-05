//! Browser WebAssembly surface for YYDB.
//!
//! Same semantic entry points as `yydb-napi` where applicable; no parallel TS parser.

#![deny(missing_docs)]
#![deny(clippy::all)]

mod core;
mod host_js;
mod opfs;
mod opfs_io;
mod session;

pub use core::{check_schema_source, introspect_schema_json, yydb_version, SchemaCheck};
pub use opfs::{
    claim_opfs_writer, open_persistent, opfs_commit_and_sync, opfs_doctor, opfs_evict_blob,
    opfs_reopen, validate_opfs_capabilities, OpfsBlobPublication, OpfsCapabilities,
    OpfsCommittedVolume, OpfsDurableSnapshot, OpfsPersistentVolume, OpfsWriterLease,
    PersistentStorageMode,
};
pub use session::{open_persistent_session, query_memory};

use wasm_bindgen::prelude::*;

/// Library version (matches workspace `@yydb/yydb` semver).
#[wasm_bindgen(js_name = yydbVersion)]
pub fn wasm_yydb_version() -> String {
    yydb_version()
}

/// Result of validating a VOS schema document via the Rust core.
#[wasm_bindgen]
pub struct CheckSchemaResult {
    ok: bool,
    table_count: u32,
    schema_fingerprint: String,
    error: Option<String>,
}

impl From<SchemaCheck> for CheckSchemaResult {
    fn from(value: SchemaCheck) -> Self {
        Self {
            ok: value.ok,
            table_count: value.table_count,
            schema_fingerprint: value.schema_fingerprint,
            error: value.error,
        }
    }
}

#[wasm_bindgen]
impl CheckSchemaResult {
    /// Whether the schema document parsed and validated.
    #[wasm_bindgen(getter)]
    pub fn ok(&self) -> bool {
        self.ok
    }

    /// Number of tables declared in the schema.
    #[wasm_bindgen(getter, js_name = tableCount)]
    pub fn table_count(&self) -> u32 {
        self.table_count
    }

    /// Stable fingerprint of the validated schema.
    #[wasm_bindgen(getter, js_name = schemaFingerprint)]
    pub fn schema_fingerprint(&self) -> String {
        self.schema_fingerprint.clone()
    }

    /// Validation error message when [`Self::ok`] is false.
    #[wasm_bindgen(getter)]
    pub fn error(&self) -> Option<String> {
        self.error.clone()
    }
}

/// Parse and validate schema source (same semantics as `Connection::ensure_schema`).
#[wasm_bindgen(js_name = checkSchema)]
pub fn check_schema(source: &str) -> CheckSchemaResult {
    check_schema_source(source).into()
}

/// Read-only schema introspection JSON.
#[wasm_bindgen(js_name = introspectSchema)]
pub fn introspect_schema(source: &str) -> String {
    introspect_schema_json(source)
}

/// Execute a VOS query on a fresh in-memory database; returns JSON `{ ok, rows, error }`.
#[wasm_bindgen(js_name = queryMemory)]
pub fn query_memory_js(source: &str) -> String {
    query_memory(source)
}

pub use session::{MemorySession, PersistentSession};
