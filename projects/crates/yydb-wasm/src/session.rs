//! Stateful in-memory YYDB session for browser WASM hosts.

use wasm_bindgen::prelude::*;
use yydb::{Connection, Error};

use crate::core::{execute_result_json, unit_result_json};

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
    pub fn new() -> Result<MemorySession, JsValue> {
        Connection::open_in_memory()
            .map(|conn| Self { conn, closed: false })
            .map_err(|err| JsValue::from_str(&err.to_string()))
    }

    /// Persist the database-truth VOS schema document.
    #[wasm_bindgen(js_name = ensureSchema)]
    pub fn ensure_schema(&self, document: &str) -> String {
        if self.closed {
            return unit_result_json(Err(Error::Unsupported("session closed")));
        }
        unit_result_json(self.conn.ensure_schema(document))
    }

    /// VOS read pipeline (for example `User.filter(x => x.active).collect()`).
    #[wasm_bindgen]
    pub fn query(&self, source: &str) -> String {
        if self.closed {
            return execute_result_json(Err(Error::Unsupported("session closed")));
        }
        execute_result_json(self.conn.query(source))
    }

    /// Unit-valued VOS write programs (for example `User { … }.insert()`).
    #[wasm_bindgen]
    pub fn execute(&self, source: &str) -> String {
        if self.closed {
            return unit_result_json(Err(Error::Unsupported("session closed")));
        }
        unit_result_json(self.conn.execute(source))
    }

    #[wasm_bindgen]
    pub fn close(&mut self) {
        self.closed = true;
    }
}
