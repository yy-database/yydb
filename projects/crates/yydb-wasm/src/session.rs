//! Stateful YYDB sessions for browser WASM hosts.

use js_sys::Function;
use wasm_bindgen::prelude::*;
use yydb::{Connection, Error, protocol as wire};

use crate::{
    core::{
        compare_exchange_json, connection_info, kv_get_json, query_rows_json, scalar_call_json, scan_prefix_json,
        schema_get_json, unit_result_json,
    },
    host_js::JsHostAdapter,
    opfs::{OpfsCapabilities, OpfsPersistentVolume, open_persistent},
};

fn closed_session_error() -> Error {
    Error::SessionClosed
}

/// Parse a write-batch JSON body: `[{ "key": "...", "value": bytes | null }, ...]`.
fn parse_write_batch(body: &[u8]) -> yydb::Result<Vec<(String, Option<Vec<u8>>)>> {
    let array: serde_json::Value = serde_json::from_slice(body)
        .map_err(|_| Error::Protocol { message: "writeBatch body must be a JSON array".to_string() })?;
    let arr =
        array.as_array().ok_or_else(|| Error::Protocol { message: "writeBatch body must be a JSON array".to_string() })?;
    arr.iter()
        .map(|entry| {
            let obj = entry
                .as_object()
                .ok_or_else(|| Error::Protocol { message: "writeBatch entry must be an object".to_string() })?;
            let key = obj
                .get("key")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Protocol { message: "writeBatch entry missing string `key`".to_string() })?
                .to_owned();
            let value = match obj.get("value") {
                Some(serde_json::Value::Null) | None => None,
                Some(v) => {
                    let bytes: Vec<u8> = v
                        .as_array()
                        .and_then(|arr| arr.iter().map(|x| x.as_i64().and_then(|n| u8::try_from(n).ok())).collect())
                        .ok_or_else(|| Error::Protocol {
                            message: "writeBatch `value` must be null or a byte array".to_string(),
                        })?;
                    Some(bytes)
                }
            };
            Ok((key, value))
        })
        .collect()
}

/// Parse a rename-map JSON body (same shape as the N-API layer accepts).
/// Returns `Ok` with an empty RenameMap when `json` is `None`, empty, or `"null"`.
fn parse_rename_map(json: Option<&str>) -> yydb::Result<yydb::catalog::RenameMap> {
    let Some(json) = json
    else {
        return Ok(yydb::catalog::RenameMap::default());
    };
    if json.is_empty() || json == "null" {
        return Ok(yydb::catalog::RenameMap::default());
    }
    let arr: Vec<serde_json::Value> = serde_json::from_str(json)
        .map_err(|_| Error::Protocol { message: "migrateSchema renames must be a JSON array".to_string() })?;
    let mut renames = yydb::catalog::RenameMap::default();
    for entry in arr {
        let obj = entry
            .as_object()
            .ok_or_else(|| Error::Protocol { message: "migrateSchema rename entry must be an object".to_string() })?;
        if let (Some(from), Some(to)) =
            (obj.get("fromType").and_then(|v| v.as_str()), obj.get("toType").and_then(|v| v.as_str()))
        {
            renames.types.insert(from.to_owned(), to.to_owned());
        }
        else if let (Some(type_name), Some(from_field), Some(to_field)) = (
            obj.get("type").and_then(|v| v.as_str()),
            obj.get("fromField").and_then(|v| v.as_str()),
            obj.get("toField").and_then(|v| v.as_str()),
        ) {
            renames.fields.insert(
                yydb::catalog::FieldPath { type_name: type_name.to_owned(), field_name: from_field.to_owned() },
                to_field.to_owned(),
            );
        }
        else {
            return Err(Error::Protocol {
                message: "migrateSchema entry needs {fromType,toType} or {type,fromField,toField}".to_string(),
            });
        }
    }
    Ok(renames)
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
        Ok((name, version, args)) => scalar_call_json(conn.call_scalar_version(&name, version, &args)),
        Err(error) => scalar_call_json(Err(error)),
    }
}

fn open_persistent_connection(volume: &OpfsPersistentVolume) -> yydb::Result<Connection> {
    let conn = Connection::open_in_memory()?;
    if let Some(bytes) = volume.read_main()? {
        conn.load_main_snapshot(&bytes)?;
    }
    Ok(conn)
}

fn persist_persistent_connection(conn: &Connection, volume: &OpfsPersistentVolume) -> yydb::Result<()> {
    let bytes = conn.main_snapshot()?;
    volume.publish_main(&bytes)?;
    Ok(())
}

fn run_ensure_schema_persistent(conn: &Connection, volume: &OpfsPersistentVolume, closed: bool, document: &str) -> String {
    if closed {
        return unit_result_json(Err(closed_session_error()));
    }
    match conn.ensure_schema(document) {
        Ok(()) => unit_result_json(persist_persistent_connection(conn, volume)),
        Err(error) => unit_result_json(Err(error)),
    }
}

fn run_put_persistent(conn: &Connection, volume: &OpfsPersistentVolume, closed: bool, key: &str, value: &[u8]) -> String {
    if closed {
        return unit_result_json(Err(closed_session_error()));
    }
    match conn.put(key, value) {
        Ok(()) => unit_result_json(persist_persistent_connection(conn, volume)),
        Err(error) => unit_result_json(Err(error)),
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
    /// Open a fresh in-memory database session.
    #[wasm_bindgen(constructor)]
    pub fn new() -> std::result::Result<MemorySession, JsValue> {
        Connection::open_in_memory().map(|conn| Self { conn, closed: false }).map_err(|err| JsValue::from_str(&err.to_string()))
    }

    /// Persist the database-truth VOS schema document.
    #[wasm_bindgen(js_name = ensureSchema)]
    pub fn ensure_schema(&self, document: &str) -> String {
        run_ensure_schema(&self.conn, self.closed, document)
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

    /// Remove a record by key. Returns JSON `{ ok, rows, error }`.
    #[wasm_bindgen(js_name = delete)]
    pub fn delete(&self, key: &str) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        unit_result_json(self.conn.write_batch(&[(key.to_owned(), None)]))
    }

    /// Scan keys in lexical order with a prefix, optional continuation cursor, and limit.
    /// Returns JSON `{ ok, rows: [{ key, value }], error }`.
    #[wasm_bindgen(js_name = scanPrefix)]
    pub fn scan_prefix(&self, prefix: &str, after: Option<String>, limit: u32) -> String {
        if self.closed {
            return scan_prefix_json(Err(closed_session_error()));
        }
        scan_prefix_json(self.conn.scan_prefix(prefix, after.as_deref(), limit as usize))
    }

    /// Commit a batch of writes or deletes atomically.
    /// Body is a JSON array: `[{ "key": "...", "value": bytes | null }, ...]`.
    /// `null` value deletes. Returns JSON `{ ok, rows, error }`.
    #[wasm_bindgen(js_name = writeBatch)]
    pub fn write_batch(&self, body: &[u8]) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        match parse_write_batch(body) {
            Ok(changes) => unit_result_json(self.conn.write_batch(&changes)),
            Err(err) => unit_result_json(Err(err)),
        }
    }

    /// Compare-and-swap: replace or delete `key` only if it currently matches `expected`.
    /// Both `expected` and `replacement` are either bytes or null (null means absent / delete).
    /// Returns JSON `{ ok, success, error }` where `success=false` means CAS lost the race.
    #[wasm_bindgen(js_name = compareExchange)]
    pub fn compare_exchange(&self, key: &str, expected: Option<Vec<u8>>, replacement: Option<Vec<u8>>) -> String {
        if self.closed {
            return compare_exchange_json(Err(closed_session_error()));
        }
        compare_exchange_json(self.conn.compare_exchange(key, expected.as_deref(), replacement.as_deref()))
    }

    /// Execute a VOS read pipeline and return the resulting rows.
    /// Returns JSON `{ ok, rows: [{ field: value, ... }], error }`.
    #[wasm_bindgen(js_name = queryVos)]
    pub fn query_vos(&self, source: &str) -> String {
        if self.closed {
            return query_rows_json(Err(closed_session_error()));
        }
        query_rows_json(self.conn.query_vos(source))
    }

    /// Execute a VOS write program (INSERT / schema macros).
    /// Returns JSON `{ ok, rows, error }`.
    #[wasm_bindgen(js_name = executeVosWrite)]
    pub fn execute_vos_write(&self, source: &str) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        unit_result_json(self.conn.execute_vos_write(source))
    }

    /// Publish a new schema version with an optional rename map.
    ///
    /// `renames_json` is a JSON array of rename entries (same shape as the
    /// N-API layer accepts), or an empty string / `"null"` when there are no
    /// renames. Returns JSON `{ ok, error }`.
    #[wasm_bindgen(js_name = migrateSchema)]
    pub fn migrate_schema(&self, document: &str, renames_json: Option<String>) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        let renames = parse_rename_map(renames_json.as_deref());
        unit_result_json(renames.and_then(|r| self.conn.migrate_schema(document, &r)))
    }

    /// Begin an explicit transaction.
    #[wasm_bindgen(js_name = begin)]
    pub fn begin(&self) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        unit_result_json(self.conn.begin())
    }

    /// Commit the current transaction.
    #[wasm_bindgen(js_name = commit)]
    pub fn commit(&self) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        unit_result_json(self.conn.commit())
    }

    /// Roll back the current transaction.
    #[wasm_bindgen(js_name = rollback)]
    pub fn rollback(&self) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        unit_result_json(self.conn.rollback())
    }

    /// Fold the WAL into the main file (memory sessions are a no-op after flush).
    #[wasm_bindgen(js_name = checkpoint)]
    pub fn checkpoint(&self) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        unit_result_json(self.conn.checkpoint())
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

    /// Mark the session closed; further calls return closed-session errors.
    #[wasm_bindgen]
    pub fn close(&mut self) {
        self.closed = true;
    }
}

/// Open a persistent OPFS-backed session with the supplied capability profile.
pub fn open_persistent_session(path: &str, caps: &OpfsCapabilities) -> yydb::Result<PersistentSession> {
    let volume = open_persistent(path, caps)?;
    let conn = open_persistent_connection(&volume)?;
    Ok(PersistentSession { path: path.to_string(), conn, volume: Some(volume), closed: false })
}

/// Stateful OPFS-backed database session with `YDPG` main-file hydration and publish.
#[wasm_bindgen]
pub struct PersistentSession {
    path: String,
    conn: Connection,
    volume: Option<OpfsPersistentVolume>,
    closed: bool,
}

#[wasm_bindgen]
impl PersistentSession {
    /// Open `path` using the full capability profile inferred from its suffix.
    #[wasm_bindgen(constructor)]
    pub fn new(path: &str) -> std::result::Result<PersistentSession, JsValue> {
        let caps =
            if path.ends_with(".yydx") { OpfsCapabilities::full_yydx_profile() } else { OpfsCapabilities::full_yydb_profile() };
        open_persistent_session(path, &caps).map_err(|err| JsValue::from_str(&err.to_string()))
    }

    /// Logical OPFS database path (for example `app.yydb`).
    #[wasm_bindgen(getter)]
    pub fn path(&self) -> String {
        self.path.clone()
    }

    /// Persist the database-truth VOS schema document.
    #[wasm_bindgen(js_name = ensureSchema)]
    pub fn ensure_schema(&self, document: &str) -> String {
        match self.volume.as_ref() {
            Some(volume) => run_ensure_schema_persistent(&self.conn, volume, self.closed, document),
            None => unit_result_json(Err(closed_session_error())),
        }
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
        match self.volume.as_ref() {
            Some(volume) => run_put_persistent(&self.conn, volume, self.closed, key, value),
            None => unit_result_json(Err(closed_session_error())),
        }
    }

    /// Remove a record by key, then persist the snapshot.
    #[wasm_bindgen(js_name = delete)]
    pub fn delete(&self, key: &str) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        match self.volume.as_ref() {
            Some(volume) => match self.conn.write_batch(&[(key.to_owned(), None)]) {
                Ok(()) => unit_result_json(persist_persistent_connection(&self.conn, volume)),
                Err(error) => unit_result_json(Err(error)),
            },
            None => unit_result_json(Err(closed_session_error())),
        }
    }

    /// Scan keys in lexical order; no persistence needed (read-only).
    #[wasm_bindgen(js_name = scanPrefix)]
    pub fn scan_prefix(&self, prefix: &str, after: Option<String>, limit: u32) -> String {
        if self.closed {
            return scan_prefix_json(Err(closed_session_error()));
        }
        scan_prefix_json(self.conn.scan_prefix(prefix, after.as_deref(), limit as usize))
    }

    /// Commit a batch atomically, then persist the snapshot.
    #[wasm_bindgen(js_name = writeBatch)]
    pub fn write_batch(&self, body: &[u8]) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        let changes = match parse_write_batch(body) {
            Ok(c) => c,
            Err(err) => return unit_result_json(Err(err)),
        };
        match self.volume.as_ref() {
            Some(volume) => match self.conn.write_batch(&changes) {
                Ok(()) => unit_result_json(persist_persistent_connection(&self.conn, volume)),
                Err(error) => unit_result_json(Err(error)),
            },
            None => unit_result_json(Err(closed_session_error())),
        }
    }

    /// Compare-and-swap, then persist the snapshot when the CAS succeeds.
    #[wasm_bindgen(js_name = compareExchange)]
    pub fn compare_exchange(&self, key: &str, expected: Option<Vec<u8>>, replacement: Option<Vec<u8>>) -> String {
        if self.closed {
            return compare_exchange_json(Err(closed_session_error()));
        }
        match self.conn.compare_exchange(key, expected.as_deref(), replacement.as_deref()) {
            Ok(true) => match self.volume.as_ref() {
                Some(volume) => compare_exchange_json(persist_persistent_connection(&self.conn, volume).map(|_| true)),
                None => compare_exchange_json(Err(closed_session_error())),
            },
            Ok(false) => compare_exchange_json(Ok(false)),
            Err(err) => compare_exchange_json(Err(err)),
        }
    }

    /// Execute a VOS read pipeline; read-only, no persistence needed.
    #[wasm_bindgen(js_name = queryVos)]
    pub fn query_vos(&self, source: &str) -> String {
        if self.closed {
            return query_rows_json(Err(closed_session_error()));
        }
        query_rows_json(self.conn.query_vos(source))
    }

    /// Execute a VOS write program, then persist the snapshot on success.
    #[wasm_bindgen(js_name = executeVosWrite)]
    pub fn execute_vos_write(&self, source: &str) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        match self.volume.as_ref() {
            Some(volume) => match self.conn.execute_vos_write(source) {
                Ok(()) => unit_result_json(persist_persistent_connection(&self.conn, volume)),
                Err(error) => unit_result_json(Err(error)),
            },
            None => unit_result_json(Err(closed_session_error())),
        }
    }

    /// Publish a new schema version with an optional rename map, then persist.
    #[wasm_bindgen(js_name = migrateSchema)]
    pub fn migrate_schema(&self, document: &str, renames_json: Option<String>) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        let Ok(renames) = parse_rename_map(renames_json.as_deref())
        else {
            return unit_result_json(Err(Error::Protocol { message: "invalid renames JSON".to_string() }));
        };
        match self.volume.as_ref() {
            Some(volume) => match self.conn.migrate_schema(document, &renames) {
                Ok(()) => unit_result_json(persist_persistent_connection(&self.conn, volume)),
                Err(error) => unit_result_json(Err(error)),
            },
            None => unit_result_json(Err(closed_session_error())),
        }
    }

    /// Begin an explicit transaction (in-memory only, no persistence needed).
    #[wasm_bindgen(js_name = begin)]
    pub fn begin(&self) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        unit_result_json(self.conn.begin())
    }

    /// Commit the current transaction and persist the snapshot.
    #[wasm_bindgen(js_name = commit)]
    pub fn commit(&self) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        match self.volume.as_ref() {
            Some(volume) => match self.conn.commit() {
                Ok(()) => unit_result_json(persist_persistent_connection(&self.conn, volume)),
                Err(error) => unit_result_json(Err(error)),
            },
            None => unit_result_json(Err(closed_session_error())),
        }
    }

    /// Roll back the current transaction and persist the rolled-back snapshot.
    #[wasm_bindgen(js_name = rollback)]
    pub fn rollback(&self) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        match self.volume.as_ref() {
            Some(volume) => match self.conn.rollback() {
                Ok(()) => unit_result_json(persist_persistent_connection(&self.conn, volume)),
                Err(error) => unit_result_json(Err(error)),
            },
            None => unit_result_json(Err(closed_session_error())),
        }
    }

    /// Run checkpoint (WAL fold) and persist the resulting snapshot.
    #[wasm_bindgen(js_name = checkpoint)]
    pub fn checkpoint(&self) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        match self.volume.as_ref() {
            Some(volume) => match self.conn.checkpoint() {
                Ok(()) => unit_result_json(persist_persistent_connection(&self.conn, volume)),
                Err(error) => unit_result_json(Err(error)),
            },
            None => unit_result_json(Err(closed_session_error())),
        }
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
        connection_info(&self.conn, Some(&self.path))
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
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        match self.volume.as_ref() {
            Some(volume) => match wire::decode_micro_register(body) {
                Ok(definition) => match self.conn.register_host_micro(definition) {
                    Ok(()) => unit_result_json(persist_persistent_connection(&self.conn, volume)),
                    Err(error) => unit_result_json(Err(error)),
                },
                Err(error) => unit_result_json(Err(error)),
            },
            None => unit_result_json(Err(closed_session_error())),
        }
    }

    /// Invoke a scalar UDF from a wire `ScalarCall` body. Returns JSON `{ ok, value, error }`.
    #[wasm_bindgen(js_name = callScalar)]
    pub fn call_scalar(&self, body: &[u8]) -> String {
        run_call_scalar(&self.conn, self.closed, body)
    }

    /// Flush pending durable state, then mark the session closed.
    #[wasm_bindgen]
    pub fn close(&mut self) {
        if !self.closed {
            if let Some(volume) = self.volume.take() {
                let _ = persist_persistent_connection(&self.conn, &volume);
            }
            self.closed = true;
        }
    }
}
