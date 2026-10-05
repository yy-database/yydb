//! Browser WebAssembly surface for YYDB.
//!
//! Same semantic entry points as `yydb-napi` where applicable; no parallel TS parser.

#![warn(missing_docs)]
#![deny(clippy::all)]

mod core;
mod opfs;
mod session;

pub use core::{check_schema_source, introspect_schema_json, yydb_version, SchemaCheck};
pub use opfs::{
    claim_opfs_writer, open_persistent, validate_opfs_capabilities, OpfsCapabilities,
    OpfsCommittedVolume, OpfsWriterLease, PersistentStorageMode,
};
pub use session::query_memory;

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
    engine_version: String,
    error: Option<String>,
}

impl From<SchemaCheck> for CheckSchemaResult {
    fn from(value: SchemaCheck) -> Self {
        Self {
            ok: value.ok,
            table_count: value.table_count,
            schema_fingerprint: value.schema_fingerprint,
            engine_version: value.engine_version,
            error: value.error,
        }
    }
}

#[wasm_bindgen]
impl CheckSchemaResult {
    #[wasm_bindgen(getter)]
    pub fn ok(&self) -> bool {
        self.ok
    }

    #[wasm_bindgen(getter, js_name = tableCount)]
    pub fn table_count(&self) -> u32 {
        self.table_count
    }

    #[wasm_bindgen(getter, js_name = schemaFingerprint)]
    pub fn schema_fingerprint(&self) -> String {
        self.schema_fingerprint.clone()
    }

    #[wasm_bindgen(getter, js_name = engineVersion)]
    pub fn engine_version(&self) -> String {
        self.engine_version.clone()
    }

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

pub use session::MemorySession;
