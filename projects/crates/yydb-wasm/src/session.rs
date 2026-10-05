//! Stateful YYDB sessions for browser WASM hosts.

use wasm_bindgen::prelude::*;
use yydb::{Connection, Error};

use crate::core::{execute_result_json, unit_result_json};
use crate::opfs::{open_persistent, OpfsCapabilities, OpfsPersistentVolume};

fn closed_session_error() -> Error {
    Error::Unsupported("session closed")
}

fn run_query(conn: &Connection, closed: bool, source: &str) -> String {
    if closed {
        return execute_result_json(Err(closed_session_error()));
    }
    execute_result_json(conn.query(source))
}

fn run_execute(conn: &Connection, closed: bool, source: &str) -> String {
    if closed {
        return unit_result_json(Err(closed_session_error()));
    }
    unit_result_json(conn.execute(source))
}

fn run_ensure_schema(conn: &Connection, closed: bool, document: &str) -> String {
    if closed {
        return unit_result_json(Err(closed_session_error()));
    }
    unit_result_json(conn.ensure_schema(document))
}

/// Execute a VOS query on a fresh in-memory database (stateless helper).
pub fn query_memory(source: &str) -> String {
    match Connection::open_in_memory() {
        Ok(conn) => execute_result_json(conn.query(source)),
        Err(err) => unit_result_json(Err(err)),
    }
}

/// Stateful in-memory database session.
#[wasm_bindgen]
pub struct MemorySession {
    conn: Connection,
    closed: bool,
}

#[wasm_bindgen]
impl MemorySession {
    #[wasm_bindgen(constructor)]
    pub fn new() -> std::result::Result<MemorySession, JsValue> {
        Connection::open_in_memory()
            .map(|conn| Self {
                conn,
                closed: false,
            })
            .map_err(|err| JsValue::from_str(&err.to_string()))
    }

    /// Persist the database-truth VOS schema document.
    #[wasm_bindgen(js_name = ensureSchema)]
    pub fn ensure_schema(&self, document: &str) -> String {
        run_ensure_schema(&self.conn, self.closed, document)
    }

    /// VOS read pipeline (for example `User.filter(x => x.active).collect()`).
    #[wasm_bindgen]
    pub fn query(&self, source: &str) -> String {
        run_query(&self.conn, self.closed, source)
    }

    /// Unit-valued VOS write programs (for example `User { … }.insert()`).
    #[wasm_bindgen]
    pub fn execute(&self, source: &str) -> String {
        run_execute(&self.conn, self.closed, source)
    }

    #[wasm_bindgen]
    pub fn close(&mut self) {
        self.closed = true;
    }
}

/// Open a persistent OPFS-backed session with the supplied capability profile.
pub fn open_persistent_session(
    path: &str,
    caps: &OpfsCapabilities,
) -> yydb::Result<PersistentSession> {
    let volume = open_persistent(path, caps)?;
    Ok(PersistentSession {
        conn: Connection::open_in_memory()?,
        volume,
        closed: false,
    })
}

/// Stateful OPFS-backed database session (in-memory engine with logical OPFS volume).
///
/// VOS execution uses an in-memory `Connection` until format bytes can be hydrated from OPFS.
#[wasm_bindgen]
pub struct PersistentSession {
    conn: Connection,
    volume: OpfsPersistentVolume,
    closed: bool,
}

#[wasm_bindgen]
impl PersistentSession {
    /// Open `path` using the full capability profile inferred from its suffix.
    #[wasm_bindgen(constructor)]
    pub fn new(path: &str) -> std::result::Result<PersistentSession, JsValue> {
        let caps = if path.ends_with(".yydx") {
            OpfsCapabilities::full_yydx_profile()
        } else {
            OpfsCapabilities::full_yydb_profile()
        };
        open_persistent_session(path, &caps).map_err(|err| JsValue::from_str(&err.to_string()))
    }

    /// Logical OPFS database path (for example `app.yydb`).
    #[wasm_bindgen(getter)]
    pub fn path(&self) -> String {
        self.volume.path().to_string()
    }

    /// Persist the database-truth VOS schema document.
    #[wasm_bindgen(js_name = ensureSchema)]
    pub fn ensure_schema(&self, document: &str) -> String {
        run_ensure_schema(&self.conn, self.closed, document)
    }

    /// VOS read pipeline.
    #[wasm_bindgen]
    pub fn query(&self, source: &str) -> String {
        run_query(&self.conn, self.closed, source)
    }

    /// Unit-valued VOS write programs.
    #[wasm_bindgen]
    pub fn execute(&self, source: &str) -> String {
        run_execute(&self.conn, self.closed, source)
    }

    #[wasm_bindgen]
    pub fn close(&mut self) {
        self.closed = true;
    }
}
