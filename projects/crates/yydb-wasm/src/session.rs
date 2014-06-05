//! Stateful YYDB sessions for browser WASM hosts.

use js_sys::Function;
use wasm_bindgen::prelude::*;
use yydb::{wire, Connection, Error};

use crate::core::{
    connection_info, execute_result_json, kv_get_json, scalar_call_json, schema_get_json,
    unit_result_json,
};
use crate::host_js::JsHostAdapter;
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

fn run_register_micro(conn: &Connection, closed: bool, body: &[u8]) -> String {
    if closed {
        return unit_result_json(Err(closed_session_error()));
    }
    match wire::decode_micro_register(body) {
        Ok(definition) => unit_result_json(conn.register_host_micro(definition)),
        Err(error) => unit_result_json(Err(error)),
    }
}

fn run_call_scalar(conn: &Connection, closed: bool, body: &[u8]) -> String {
    if closed {
        return scalar_call_json(Err(closed_session_error()));
    }
    match wire::decode_scalar_call(body) {
        Ok((name, version, args)) => {
            scalar_call_json(conn.call_scalar_version(&name, version, &args))
        }
        Err(error) => scalar_call_json(Err(error)),
    }
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

    /// Fetch a raw byte record by key. Returns JSON `{ ok, value, error }`.
    #[wasm_bindgen(js_name = get)]
    pub fn get(&self, key: &str) -> String {
        if self.closed {
            return kv_get_json(Err(closed_session_error()));
        }
        kv_get_json(self.conn.get(key))
    }

    /// Insert or replace a raw byte record. Returns JSON `{ ok, rows, error }`.
    #[wasm_bindgen(js_name = put)]
    pub fn put(&self, key: &str, value: &[u8]) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        unit_result_json(self.conn.put(key, value))
    }

    /// Current persisted schema, if any. Returns JSON `{ ok, schema, error }`.
    #[wasm_bindgen(js_name = getSchema)]
    pub fn get_schema(&self) -> String {
        if self.closed {
            return schema_get_json(Err(closed_session_error()));
        }
        schema_get_json(self.conn.schema())
    }

    /// Database info text (`path=…\\n…`), matching the YY wire `Info` reply.
    #[wasm_bindgen]
    pub fn info(&self) -> String {
        if self.closed {
            return String::new();
        }
        connection_info(&self.conn, None)
    }

    /// Install the JS host micro invoker used by [`registerMicro`](Self::register_micro).
    #[wasm_bindgen(js_name = setMicroHostInvoker)]
    pub fn set_micro_host_invoker(&self, invoker: Function) {
        if self.closed {
            return;
        }
        JsHostAdapter::install(&self.conn, invoker);
    }

    /// Register a session-local host micro from a wire `MicroRegister` body.
    #[wasm_bindgen(js_name = registerMicro)]
    pub fn register_micro(&self, body: &[u8]) -> String {
        run_register_micro(&self.conn, self.closed, body)
    }

    /// Invoke a scalar UDF from a wire `ScalarCall` body. Returns JSON `{ ok, value, error }`.
    #[wasm_bindgen(js_name = callScalar)]
    pub fn call_scalar(&self, body: &[u8]) -> String {
        run_call_scalar(&self.conn, self.closed, body)
    }

    /// Engine version string (wire `HelloOk` parity).
    #[wasm_bindgen(js_name = serverVersion)]
    pub fn server_version(&self) -> String {
        yydb::version().to_string()
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

    /// Fetch a raw byte record by key. Returns JSON `{ ok, value, error }`.
    #[wasm_bindgen(js_name = get)]
    pub fn get(&self, key: &str) -> String {
        if self.closed {
            return kv_get_json(Err(closed_session_error()));
        }
        kv_get_json(self.conn.get(key))
    }

    /// Insert or replace a raw byte record. Returns JSON `{ ok, rows, error }`.
    #[wasm_bindgen(js_name = put)]
    pub fn put(&self, key: &str, value: &[u8]) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        unit_result_json(self.conn.put(key, value))
    }

    /// Current persisted schema, if any. Returns JSON `{ ok, schema, error }`.
    #[wasm_bindgen(js_name = getSchema)]
    pub fn get_schema(&self) -> String {
        if self.closed {
            return schema_get_json(Err(closed_session_error()));
        }
        schema_get_json(self.conn.schema())
    }

    /// Database info text (`path=…\\n…`), matching the YY wire `Info` reply.
    #[wasm_bindgen]
    pub fn info(&self) -> String {
        if self.closed {
            return String::new();
        }
        connection_info(&self.conn, Some(self.volume.path()))
    }

    /// Install the JS host micro invoker used by [`registerMicro`](Self::register_micro).
    #[wasm_bindgen(js_name = setMicroHostInvoker)]
    pub fn set_micro_host_invoker(&self, invoker: Function) {
        if self.closed {
            return;
        }
        JsHostAdapter::install(&self.conn, invoker);
    }

    /// Register a session-local host micro from a wire `MicroRegister` body.
    #[wasm_bindgen(js_name = registerMicro)]
    pub fn register_micro(&self, body: &[u8]) -> String {
        run_register_micro(&self.conn, self.closed, body)
    }

    /// Invoke a scalar UDF from a wire `ScalarCall` body. Returns JSON `{ ok, value, error }`.
    #[wasm_bindgen(js_name = callScalar)]
    pub fn call_scalar(&self, body: &[u8]) -> String {
        run_call_scalar(&self.conn, self.closed, body)
    }

    /// Engine version string (wire `HelloOk` parity).
    #[wasm_bindgen(js_name = serverVersion)]
    pub fn server_version(&self) -> String {
        yydb::version().to_string()
    }

    #[wasm_bindgen]
    pub fn close(&mut self) {
        self.closed = true;
    }
}
