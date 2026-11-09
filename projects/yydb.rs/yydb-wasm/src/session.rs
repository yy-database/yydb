//! Stateful YYDB sessions for browser WASM hosts.

use std::collections::{BTreeMap, HashMap};

use database::{execution::engine::Connection, protocol::yy as wire, query::local::PreparedStatement, types::Error};
use js_sys::Function;
use wasm_bindgen::prelude::*;

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

/// wasm 边界 DTO：单个参数描述符的 JSON 形态。
///
/// 核心类型 [`database::query::local::ParameterDescriptor`] 保持 Rust 命名，
/// JS 侧的 camelCase 拼写只由本 DTO 负责。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ParameterDescriptorJson<'a> {
    name: &'a str,
    index: u32,
    slot_ty: database::query::local::SlotTy,
    usage_count: u32,
}

impl<'a> ParameterDescriptorJson<'a> {
    fn new(descriptor: &'a database::query::local::ParameterDescriptor) -> Self {
        Self {
            name: descriptor.name.as_str(),
            index: descriptor.index,
            slot_ty: descriptor.slot_type,
            usage_count: descriptor.usage_count,
        }
    }
}

/// wasm 边界 DTO：预编译语句元数据的 JSON 形态。
///
/// 字段与键名冻结 `PreparedStatement` 的公开序列化形态，核心层不再承担
/// 面向 JS 的重命名。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StatementInfoJson<'a> {
    statement_id: &'a str,
    dialect: database::query::local::Dialect,
    query_kind: database::query::local::QueryKind,
    parameters: Vec<ParameterDescriptorJson<'a>>,
    read_set: &'a [String],
    write_set: &'a [String],
    source_fingerprint: &'a str,
    source: &'a str,
    schema_fingerprint: &'a str,
    schema_revision: u64,
}

impl<'a> StatementInfoJson<'a> {
    fn new(statement: &'a PreparedStatement) -> Self {
        Self {
            statement_id: statement.statement_id.as_str(),
            dialect: statement.dialect,
            query_kind: statement.query_kind,
            parameters: statement.parameters.iter().map(ParameterDescriptorJson::new).collect(),
            read_set: statement.read_set.as_slice(),
            write_set: statement.write_set.as_slice(),
            source_fingerprint: statement.source_fingerprint.as_str(),
            source: statement.source.as_str(),
            schema_fingerprint: statement.schema_fingerprint.as_str(),
            schema_revision: statement.schema_revision,
        }
    }
}

/// 编码 `prepareJson` 的返回体：语句元数据加盖 `ok` 与会话级 `statementId`。
fn encode_statement_info(statement_id: &str, statement: &PreparedStatement) -> Result<String, serde_json::Error> {
    let mut value = serde_json::to_value(StatementInfoJson::new(statement))?;
    value["ok"] = serde_json::Value::Bool(true);
    value["statementId"] = serde_json::Value::String(statement_id.to_owned());
    Ok(value.to_string())
}

/// Parse a write-batch JSON body: `[{ "key": "...", "value": bytes | null }, ...]`.
fn parse_write_batch(body: &[u8]) -> database::types::Result<Vec<(String, Option<Vec<u8>>)>> {
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
fn parse_rename_map(json: Option<&str>) -> database::types::Result<database::execution::engine::catalog::RenameMap> {
    let Some(json) = json
    else {
        return Ok(database::execution::engine::catalog::RenameMap::default());
    };
    if json.is_empty() || json == "null" {
        return Ok(database::execution::engine::catalog::RenameMap::default());
    }
    let arr: Vec<serde_json::Value> = serde_json::from_str(json)
        .map_err(|_| Error::Protocol { message: "migrateSchema renames must be a JSON array".to_string() })?;
    let mut renames = database::execution::engine::catalog::RenameMap::default();
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
                database::execution::engine::catalog::FieldPath {
                    type_name: type_name.to_owned(),
                    field_name: from_field.to_owned(),
                },
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

/// Parse a JSON MicroRegister request:
/// `{ name, version, argTypes: ["i64"|"bool"|"text"|"null"], returnType, hostId, functionId, handleVersion, fingerprint: [32 u8s] }`.
fn parse_micro_register_json(json: &str) -> database::types::Result<database::udf::HostMicroDefinition> {
    let obj: serde_json::Value = serde_json::from_str(json)
        .map_err(|_| Error::Protocol { message: "registerMicroJson must be a JSON object".to_string() })?;
    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Protocol { message: "registerMicroJson missing string `name`".to_string() })?
        .to_owned();
    let version = obj
        .get("version")
        .and_then(|v| v.as_u64())
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| Error::Protocol { message: "registerMicroJson missing u32 `version`".to_string() })?;
    let arg_types_arr = obj
        .get("argTypes")
        .and_then(|v| v.as_array())
        .ok_or_else(|| Error::Protocol { message: "registerMicroJson missing array `argTypes`".to_string() })?;
    let arg_types: Vec<database::udf::UdfType> = arg_types_arr
        .iter()
        .map(|v| {
            let s = v
                .as_str()
                .ok_or_else(|| Error::Protocol { message: "registerMicroJson `argTypes` must be string tags".to_string() })?;
            parse_udf_type_tag(s)
        })
        .collect::<Result<_, _>>()?;
    let return_type = obj
        .get("returnType")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Protocol { message: "registerMicroJson missing string `returnType`".to_string() })?;
    let return_type = parse_udf_type_tag(return_type)?;
    let host_id = obj
        .get("hostId")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| Error::Protocol { message: "registerMicroJson missing u64 `hostId`".to_string() })?;
    let function_id = obj
        .get("functionId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Protocol { message: "registerMicroJson missing string `functionId`".to_string() })?
        .to_owned();
    let handle_version = obj
        .get("handleVersion")
        .and_then(|v| v.as_u64())
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| Error::Protocol { message: "registerMicroJson missing u32 `handleVersion`".to_string() })?;
    let fingerprint_arr = obj
        .get("fingerprint")
        .and_then(|v| v.as_array())
        .ok_or_else(|| Error::Protocol { message: "registerMicroJson missing array `fingerprint`".to_string() })?;
    let mut fingerprint = [0u8; 32];
    if fingerprint_arr.len() != 32 {
        return Err(Error::Protocol {
            message: format!("registerMicroJson `fingerprint` must be 32 bytes, got {}", fingerprint_arr.len()),
        });
    }
    for (i, v) in fingerprint_arr.iter().enumerate() {
        let byte = v
            .as_u64()
            .and_then(|n| u8::try_from(n).ok())
            .ok_or_else(|| Error::Protocol { message: "registerMicroJson `fingerprint` values must be u8".to_string() })?;
        fingerprint[i] = byte;
    }
    database::udf::HostMicroDefinition::from_scalar(
        &name,
        version,
        arg_types,
        return_type,
        host_id,
        function_id,
        handle_version,
        fingerprint,
    )
    .map_err(|_| Error::Protocol { message: "registerMicroJson failed validation".to_string() })
}

/// Parse a JSON ScalarCall request:
/// `{ name, version, args: [42, "text", true, null] }`.
fn parse_scalar_call_json(json: &str) -> database::types::Result<(String, u32, Vec<database::types::Value>)> {
    let obj: serde_json::Value = serde_json::from_str(json)
        .map_err(|_| Error::Protocol { message: "callScalarJson must be a JSON object".to_string() })?;
    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Protocol { message: "callScalarJson missing string `name`".to_string() })?
        .to_owned();
    let version = obj
        .get("version")
        .and_then(|v| v.as_u64())
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| Error::Protocol { message: "callScalarJson missing u32 `version`".to_string() })?;
    let args_arr = obj
        .get("args")
        .and_then(|v| v.as_array())
        .ok_or_else(|| Error::Protocol { message: "callScalarJson missing array `args`".to_string() })?;
    let mut args = Vec::with_capacity(args_arr.len());
    for v in args_arr {
        args.push(json_value_to_runtime(v)?);
    }
    Ok((name, version, args))
}

/// Map a JSON scalar to the YY runtime `Value` subset (used by callScalar args).
fn json_value_to_runtime(v: &serde_json::Value) -> database::types::Result<database::types::Value> {
    match v {
        serde_json::Value::Null => Ok(database::types::Value::Null),
        serde_json::Value::Bool(b) => Ok(database::types::Value::Bool(*b)),
        serde_json::Value::Number(n) => {
            // Prefer i64 when it fits exactly; fall back to Float for fractional.
            if let Some(i) = n.as_i64() {
                Ok(database::types::Value::I64(i))
            }
            else if let Some(f) = n.as_f64() {
                Ok(database::types::Value::F64(f))
            }
            else {
                Err(Error::Protocol { message: "callScalarJson number out of range".to_string() })
            }
        }
        serde_json::Value::String(s) => Ok(database::types::Value::Text(s.clone())),
        _ => Err(Error::Protocol { message: "callScalarJson args must be scalar values".to_string() }),
    }
}

/// Map a wire `UdfType` tag string back to the enum variant.
fn parse_udf_type_tag(s: &str) -> database::types::Result<database::udf::UdfType> {
    match s {
        "i64" => Ok(database::udf::UdfType::I64),
        "bool" => Ok(database::udf::UdfType::Bool),
        "text" => Ok(database::udf::UdfType::Text),
        "null" => Ok(database::udf::UdfType::Null),
        _ => Err(Error::Protocol { message: format!("unknown UDF type tag: {s}") }),
    }
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

fn run_register_micro_json(conn: &Connection, closed: bool, json: &str) -> String {
    if closed {
        return unit_result_json(Err(closed_session_error()));
    }
    match parse_micro_register_json(json) {
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

fn run_call_scalar_json(conn: &Connection, closed: bool, json: &str) -> String {
    if closed {
        return scalar_call_json(Err(closed_session_error()));
    }
    match parse_scalar_call_json(json) {
        Ok((name, version, args)) => scalar_call_json(conn.call_scalar_version(&name, version, &args)),
        Err(error) => scalar_call_json(Err(error)),
    }
}

fn open_persistent_connection(volume: &OpfsPersistentVolume) -> database::types::Result<Connection> {
    let conn = Connection::open_in_memory()?;
    if let Some(bytes) = volume.read_main()? {
        conn.load_main_snapshot(&bytes)?;
    }
    Ok(conn)
}

fn persist_persistent_connection(conn: &Connection, volume: &OpfsPersistentVolume) -> database::types::Result<()> {
    let bytes = conn.main_snapshot()?;
    volume.publish_main(&bytes)?;
    Ok(())
}

/// Check a main-file snapshot the way a restore entry must: load it into a
/// throwaway handle so page CRC32C checks run, then require a clean doctor
/// report before the snapshot is allowed to reach durable storage.
fn validate_snapshot(bytes: &[u8]) -> database::types::Result<()> {
    let probe = Connection::open_in_memory()?;
    probe.load_main_snapshot(bytes)?;
    probe.doctor().map(|_| ())
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
    stmts: HashMap<String, PreparedStatement>,
}

#[wasm_bindgen]
impl MemorySession {
    /// Open a fresh in-memory database session.
    #[wasm_bindgen(constructor)]
    pub fn new() -> std::result::Result<MemorySession, JsValue> {
        Connection::open_in_memory()
            .map(|conn| Self { conn, closed: false, stmts: HashMap::new() })
            .map_err(|err| JsValue::from_str(&err.to_string()))
    }

    /// Open a session from a main-file snapshot, verifying every page first.
    ///
    /// This is the browser restore entry: the snapshot is loaded into a
    /// throwaway handle so page CRC32C checks run, `doctor` must report no
    /// errors, and only then does the session take ownership of the pages.
    #[wasm_bindgen(js_name = restoreFromSnapshot)]
    pub fn restore_from_snapshot(bytes: &[u8]) -> std::result::Result<MemorySession, JsValue> {
        validate_snapshot(bytes).map_err(|err| JsValue::from_str(&err.to_string()))?;
        let conn = Connection::open_in_memory().map_err(|err| JsValue::from_str(&err.to_string()))?;
        conn.load_main_snapshot(bytes).map_err(|err| JsValue::from_str(&err.to_string()))?;
        Ok(Self { conn, closed: false, stmts: HashMap::new() })
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

    /// Create a named savepoint inside the open transaction.
    #[wasm_bindgen(js_name = savepoint)]
    pub fn savepoint(&self, name: &str) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        unit_result_json(self.conn.savepoint(name))
    }

    /// Roll back to a savepoint, discarding writes made after it.
    #[wasm_bindgen(js_name = rollbackTo)]
    pub fn rollback_to(&self, name: &str) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        unit_result_json(self.conn.rollback_to_savepoint(name))
    }

    /// Release a savepoint so it can no longer be rolled back to.
    #[wasm_bindgen(js_name = release)]
    pub fn release(&self, name: &str) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        unit_result_json(self.conn.release_savepoint(name))
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

    /// Register a session-local host micro from a JSON descriptor.
    ///
    /// Accepts the same shape as the TS `defineMicro` + encoded handle:
    /// `{ name, version, argTypes, returnType, hostId, functionId, handleVersion, fingerprint }`.
    /// Returns JSON `{ ok, error }`.
    #[wasm_bindgen(js_name = registerMicroJson)]
    pub fn register_micro_json(&self, json: &str) -> String {
        run_register_micro_json(&self.conn, self.closed, json)
    }

    /// Invoke a scalar UDF from JSON args.
    ///
    /// Accepts `{ name, version, args: [scalars] }` — avoids the binary
    /// `ScalarCall` body when the caller is JS. Returns JSON `{ ok, value, error }`.
    #[wasm_bindgen(js_name = callScalarJson)]
    pub fn call_scalar_json(&self, json: &str) -> String {
        run_call_scalar_json(&self.conn, self.closed, json)
    }

    /// Read-only consistency probes against the database.
    ///
    /// Returns JSON `{ ok, report, error }` where `report` matches the
    /// Node-side `DoctorReport` shape (namespace stats, checkpoint lag,
    /// orphan blobs, named issues).
    #[wasm_bindgen(js_name = doctorJson)]
    pub fn doctor_json(&self) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        match self.conn.doctor() {
            Ok(report) => serde_json::json!({ "ok": true, "report": report }).to_string(),
            Err(error) => serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
        }
    }

    /// Prepare a VOS read pipeline into a reusable query plan.
    ///
    /// Registers the plan in this session's internal registry and returns
    /// JSON `{ ok, statementId, parameters, queryKind, sourceFingerprint,
    /// readSet, writeSet, source, error }`. No schema binding required —
    /// plan building is pure parse + lower. A write program has no reusable
    /// read plan, so it is refused here and must go through the write surface.
    #[wasm_bindgen(js_name = prepareJson)]
    pub fn prepare_json(&mut self, source: &str) -> String {
        if self.closed {
            return serde_json::json!({ "ok": false, "error": closed_session_error().to_string() }).to_string();
        }
        match self.conn.prepare_vos(source) {
            Ok(stmt) => {
                if stmt.query_kind != database::query::local::QueryKind::Read {
                    let error = Error::Schema { message: "Phase 1 only executes pipelines ending in `.collect()`".into() };
                    return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string();
                }
                let id = format!("stmt-{}", self.stmts.len() + 1);
                self.stmts.insert(id.clone(), stmt.clone());
                match encode_statement_info(&id, &stmt) {
                    Ok(json) => json,
                    Err(error) => serde_json::json!({ "ok": false, "error": format!("encode statement: {error}") }).to_string(),
                }
            }
            Err(error) => serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
        }
    }

    /// Execute a previously prepared statement with optional named parameter bindings.
    ///
    /// Accepts `{ statementId: "...", params: { "name": value, ... } | null }`.
    /// Returns JSON `{ ok, rows, error }` where `rows` matches a regular
    /// `queryVos` result.
    #[wasm_bindgen(js_name = queryPreparedJson)]
    pub fn query_prepared_json(&self, json: &str) -> String {
        if self.closed {
            return serde_json::json!({ "ok": false, "error": closed_session_error().to_string() }).to_string();
        }
        let obj: serde_json::Value = match serde_json::from_str(json) {
            Ok(v) => v,
            Err(error) => {
                return serde_json::json!({ "ok": false, "error": format!("invalid queryPrepared body: {error}") }).to_string();
            }
        };
        let id = match obj.get("statementId").and_then(|v| v.as_str()) {
            Some(s) => s.to_owned(),
            None => {
                return serde_json::json!({ "ok": false, "error": "queryPrepared: missing string `statementId`" }).to_string();
            }
        };
        let stmt = match self.stmts.get(&id) {
            Some(s) => s,
            None => {
                return serde_json::json!({ "ok": false, "error": format!("prepared statement not found: {id}") }).to_string();
            }
        };
        let named: BTreeMap<String, database::types::Value> = match obj.get("params") {
            None | Some(serde_json::Value::Null) => BTreeMap::new(),
            Some(val) => {
                let obj = match val.as_object() {
                    Some(o) => o,
                    None => {
                        return serde_json::json!({ "ok": false, "error": "queryPrepared: params must be an object or null" })
                            .to_string();
                    }
                };
                let mut out = BTreeMap::new();
                for (k, v) in obj {
                    out.insert(
                        k.clone(),
                        match json_value_to_runtime(v) {
                            Ok(val) => val,
                            Err(error) => return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
                        },
                    );
                }
                out
            }
        };
        match self.conn.query_prepared(stmt, &named) {
            Ok(rows) => {
                let mapped: Vec<serde_json::Map<String, serde_json::Value>> = rows
                    .into_iter()
                    .map(|row| {
                        let mut map = serde_json::Map::new();
                        for (k, v) in row {
                            map.insert(k, value_to_json(v));
                        }
                        map
                    })
                    .collect();
                serde_json::json!({ "ok": true, "rows": mapped }).to_string()
            }
            Err(error) => serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
        }
    }

    /// Release a prepared statement's plan from the session registry. Idempotent.
    #[wasm_bindgen(js_name = closeStatement)]
    pub fn close_statement(&mut self, statement_id: &str) -> bool {
        self.stmts.remove(statement_id).is_some()
    }

    /// Mark the session closed; further calls return closed-session errors.
    #[wasm_bindgen]
    pub fn close(&mut self) {
        self.closed = true;
        self.stmts.clear();
    }
}

/// Serialize a VOS runtime Value to JSON.
fn value_to_json(v: database::types::Value) -> serde_json::Value {
    use database::types::Value;
    match v {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(b),
        Value::Text(s) => serde_json::Value::String(s),
        Value::I64(i) => serde_json::Value::Number(serde_json::Number::from(i)),
        Value::U64(u) => serde_json::Value::Number(serde_json::Number::from(u)),
        Value::F64(f) => serde_json::Number::from_f64(f).map(serde_json::Value::Number).unwrap_or(serde_json::Value::Null),
        Value::Bytes(b) => serde_json::Value::Array(
            b.into_iter().map(|x| serde_json::Value::Number(serde_json::Number::from(x as u8))).collect(),
        ),
        Value::Uuid(u) => serde_json::Value::String(u.to_string()),
        _ => serde_json::Value::Null, // Vector, Object, File, Row — WASM JSON bridge skips for now
    }
}

/// Open a persistent OPFS-backed session with the supplied capability profile.
pub fn open_persistent_session(path: &str, caps: &OpfsCapabilities) -> database::types::Result<PersistentSession> {
    let volume = open_persistent(path, caps)?;
    let conn = open_persistent_connection(&volume)?;
    Ok(PersistentSession { path: path.to_string(), conn, volume: Some(volume), closed: false, stmts: HashMap::new() })
}

/// Stateful OPFS-backed database session with `YDPG` main-file hydration and publish.
#[wasm_bindgen]
pub struct PersistentSession {
    path: String,
    conn: Connection,
    volume: Option<OpfsPersistentVolume>,
    closed: bool,
    stmts: HashMap<String, PreparedStatement>,
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

    /// Replace `path` with a verified main-file snapshot and open a session.
    ///
    /// The snapshot is validated (page CRC32C plus a doctor pass) before it is
    /// published, so a damaged backup can never overwrite a live database.
    /// The caller must match the layout: a `.yydb` snapshot restores into a
    /// `.yydb` path, a `.yydx` main file into a `.yydx` path.
    #[wasm_bindgen(js_name = restoreBackup)]
    pub fn restore_backup(path: &str, bytes: &[u8]) -> std::result::Result<PersistentSession, JsValue> {
        validate_snapshot(bytes).map_err(|err| JsValue::from_str(&err.to_string()))?;
        let caps =
            if path.ends_with(".yydx") { OpfsCapabilities::full_yydx_profile() } else { OpfsCapabilities::full_yydb_profile() };
        let volume = open_persistent(path, &caps).map_err(|err| JsValue::from_str(&err.to_string()))?;
        volume.publish_main(bytes).map_err(|err| JsValue::from_str(&err.to_string()))?;
        let conn = open_persistent_connection(&volume).map_err(|err| JsValue::from_str(&err.to_string()))?;
        Ok(Self { path: path.to_string(), conn, volume: Some(volume), closed: false, stmts: HashMap::new() })
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

    /// Create a named savepoint inside the open transaction, then persist.
    #[wasm_bindgen(js_name = savepoint)]
    pub fn savepoint(&self, name: &str) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        match self.volume.as_ref() {
            Some(volume) => match self.conn.savepoint(name) {
                Ok(()) => unit_result_json(persist_persistent_connection(&self.conn, volume)),
                Err(error) => unit_result_json(Err(error)),
            },
            None => unit_result_json(Err(closed_session_error())),
        }
    }

    /// Roll back to a savepoint, then persist the rolled-back snapshot.
    #[wasm_bindgen(js_name = rollbackTo)]
    pub fn rollback_to(&self, name: &str) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        match self.volume.as_ref() {
            Some(volume) => match self.conn.rollback_to_savepoint(name) {
                Ok(()) => unit_result_json(persist_persistent_connection(&self.conn, volume)),
                Err(error) => unit_result_json(Err(error)),
            },
            None => unit_result_json(Err(closed_session_error())),
        }
    }

    /// Release a savepoint, then persist.
    #[wasm_bindgen(js_name = release)]
    pub fn release(&self, name: &str) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        match self.volume.as_ref() {
            Some(volume) => match self.conn.release_savepoint(name) {
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

    /// Register a session-local host micro from a JSON descriptor (Persistent session variant).
    #[wasm_bindgen(js_name = registerMicroJson)]
    pub fn register_micro_json(&self, json: &str) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        match self.volume.as_ref() {
            Some(volume) => match parse_micro_register_json(json) {
                Ok(definition) => match self.conn.register_host_micro(definition) {
                    Ok(()) => unit_result_json(persist_persistent_connection(&self.conn, volume)),
                    Err(error) => unit_result_json(Err(error)),
                },
                Err(error) => unit_result_json(Err(error)),
            },
            None => unit_result_json(Err(closed_session_error())),
        }
    }

    /// Invoke a scalar UDF from JSON args (Persistent session variant).
    #[wasm_bindgen(js_name = callScalarJson)]
    pub fn call_scalar_json(&self, json: &str) -> String {
        run_call_scalar_json(&self.conn, self.closed, json)
    }

    /// Read-only consistency probes against the persistent database.
    ///
    /// Returns JSON `{ ok, report, error }` matching Node-side DoctorReport shape.
    #[wasm_bindgen(js_name = doctorJson)]
    pub fn doctor_json(&self) -> String {
        if self.closed {
            return unit_result_json(Err(closed_session_error()));
        }
        match self.conn.doctor() {
            Ok(report) => serde_json::json!({ "ok": true, "report": report }).to_string(),
            Err(error) => serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
        }
    }

    /// Prepare a VOS read pipeline into a reusable query plan (Persistent session variant).
    /// A write program has no reusable read plan, so it is refused here and
    /// must go through the write surface.
    #[wasm_bindgen(js_name = prepareJson)]
    pub fn prepare_json(&mut self, source: &str) -> String {
        if self.closed {
            return serde_json::json!({ "ok": false, "error": closed_session_error().to_string() }).to_string();
        }
        match self.conn.prepare_vos(source) {
            Ok(stmt) => {
                if stmt.query_kind != database::query::local::QueryKind::Read {
                    let error = Error::Schema { message: "Phase 1 only executes pipelines ending in `.collect()`".into() };
                    return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string();
                }
                let id = format!("stmt-{}", self.stmts.len() + 1);
                self.stmts.insert(id.clone(), stmt.clone());
                match encode_statement_info(&id, &stmt) {
                    Ok(json) => json,
                    Err(error) => serde_json::json!({ "ok": false, "error": format!("encode statement: {error}") }).to_string(),
                }
            }
            Err(error) => serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
        }
    }

    /// Execute a previously prepared statement (Persistent session variant).
    #[wasm_bindgen(js_name = queryPreparedJson)]
    pub fn query_prepared_json(&self, json: &str) -> String {
        if self.closed {
            return serde_json::json!({ "ok": false, "error": closed_session_error().to_string() }).to_string();
        }
        let obj: serde_json::Value = match serde_json::from_str(json) {
            Ok(v) => v,
            Err(error) => {
                return serde_json::json!({ "ok": false, "error": format!("invalid queryPrepared body: {error}") }).to_string();
            }
        };
        let id = match obj.get("statementId").and_then(|v| v.as_str()) {
            Some(s) => s.to_owned(),
            None => {
                return serde_json::json!({ "ok": false, "error": "queryPrepared: missing string `statementId`" }).to_string();
            }
        };
        let stmt = match self.stmts.get(&id) {
            Some(s) => s,
            None => {
                return serde_json::json!({ "ok": false, "error": format!("prepared statement not found: {id}") }).to_string();
            }
        };
        let named: BTreeMap<String, database::types::Value> = match obj.get("params") {
            None | Some(serde_json::Value::Null) => BTreeMap::new(),
            Some(val) => {
                let obj = match val.as_object() {
                    Some(o) => o,
                    None => {
                        return serde_json::json!({ "ok": false, "error": "queryPrepared: params must be an object or null" })
                            .to_string();
                    }
                };
                let mut out = BTreeMap::new();
                for (k, v) in obj {
                    out.insert(
                        k.clone(),
                        match json_value_to_runtime(v) {
                            Ok(val) => val,
                            Err(error) => return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
                        },
                    );
                }
                out
            }
        };
        match self.conn.query_prepared(stmt, &named) {
            Ok(rows) => {
                let mapped: Vec<serde_json::Map<String, serde_json::Value>> = rows
                    .into_iter()
                    .map(|row| {
                        let mut map = serde_json::Map::new();
                        for (k, v) in row {
                            map.insert(k, value_to_json(v));
                        }
                        map
                    })
                    .collect();
                serde_json::json!({ "ok": true, "rows": mapped }).to_string()
            }
            Err(error) => serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
        }
    }

    /// Release a prepared statement's plan from this persistent session's registry. Idempotent.
    #[wasm_bindgen(js_name = closeStatement)]
    pub fn close_statement(&mut self, statement_id: &str) -> bool {
        self.stmts.remove(statement_id).is_some()
    }

    /// Flush pending durable state, then mark the session closed.
    #[wasm_bindgen]
    pub fn close(&mut self) {
        if !self.closed {
            if let Some(volume) = self.volume.take() {
                let _ = persist_persistent_connection(&self.conn, &volume);
            }
            self.stmts.clear();
            self.closed = true;
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests for JSON bridge parsers
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use database::types::Value;

    fn ok_register_json() -> String {
        let fingerprint = [0u8; 32];
        let fp_arr: Vec<u8> = fingerprint.to_vec();
        serde_json::json!({
            "name": "double",
            "version": 1,
            "argTypes": ["i64"],
            "returnType": "i64",
            "hostId": 7,
            "functionId": "1",
            "handleVersion": 1,
            "fingerprint": fp_arr,
        })
        .to_string()
    }

    fn ok_call_json() -> String {
        serde_json::json!({
            "name": "double",
            "version": 1,
            "args": [21],
        })
        .to_string()
    }

    #[test]
    fn parses_micro_register_json_successfully() {
        let definition = parse_micro_register_json(&ok_register_json()).expect("valid register json");
        assert_eq!(definition.identity.name(), "double");
        assert_eq!(definition.identity.version, 1);
        assert_eq!(definition.signature.args, vec![database::udf::UdfType::I64]);
        assert_eq!(definition.signature.returns, database::udf::UdfType::I64);
        assert_eq!(definition.handle.host_id, 7);
        assert_eq!(definition.handle.function_id, "1");
        assert_eq!(definition.handle.version, 1);
        assert_eq!(definition.fingerprint, [0u8; 32]);
    }

    #[test]
    fn rejects_micro_register_missing_field() {
        let mut obj = serde_json::json!({
            "name": "double",
            "version": 1,
            "argTypes": ["i64"],
            "returnType": "i64",
            "hostId": 7,
            "functionId": "1",
            "handleVersion": 1,
            "fingerprint": vec![0u8; 32],
        });
        let map = obj.as_object_mut().unwrap();
        map.remove("name");
        let err = parse_micro_register_json(&obj.to_string()).expect_err("should fail");
        assert!(format!("{err:?}").contains("registerMicroJson"));
    }

    #[test]
    fn rejects_micro_register_invalid_type_tag() {
        let json = serde_json::json!({
            "name": "fancy",
            "version": 1,
            "argTypes": ["i128"],
            "returnType": "i64",
            "hostId": 1,
            "functionId": "1",
            "handleVersion": 1,
            "fingerprint": vec![0u8; 32],
        })
        .to_string();
        let err = parse_micro_register_json(&json).expect_err("should fail");
        assert!(format!("{err:?}").contains("unknown UDF type tag"));
    }

    #[test]
    fn rejects_micro_register_wrong_fingerprint_length() {
        let json = serde_json::json!({
            "name": "short",
            "version": 1,
            "argTypes": [],
            "returnType": "bool",
            "hostId": 1,
            "functionId": "1",
            "handleVersion": 1,
            "fingerprint": vec![0u8; 16],
        })
        .to_string();
        let err = parse_micro_register_json(&json).expect_err("should fail");
        assert!(format!("{err:?}").contains("32 bytes"));
    }

    #[test]
    fn parses_scalar_call_json_with_number_to_i64() {
        let (name, version, args) = parse_scalar_call_json(&ok_call_json()).expect("valid call json");
        assert_eq!(name, "double");
        assert_eq!(version, 1);
        assert_eq!(args, vec![Value::I64(21)]);
    }

    #[test]
    fn parses_scalar_call_json_with_various_scalars() {
        let json = serde_json::json!({
            "name": "mixed",
            "version": 1,
            "args": [42, "hello", true, null, 3.14],
        })
        .to_string();
        let (_, _, args) = parse_scalar_call_json(&json).expect("ok");
        assert_eq!(args[0], Value::I64(42));
        assert_eq!(args[1], Value::Text("hello".to_owned()));
        assert_eq!(args[2], Value::Bool(true));
        assert_eq!(args[3], Value::Null);
        assert!(matches!(args[4], Value::F64(_)));
    }

    #[test]
    fn rejects_scalar_call_missing_field() {
        let json = r#"{"name":"only"}"#;
        let err = parse_scalar_call_json(json).expect_err("should fail");
        assert!(format!("{err:?}").contains("callScalarJson"));
    }

    #[test]
    fn rejects_scalar_call_non_scalar_arg() {
        let json = serde_json::json!({
            "name": "bad",
            "version": 1,
            "args": [{"nested": true}],
        })
        .to_string();
        let err = parse_scalar_call_json(&json).expect_err("should fail");
        assert!(format!("{err:?}").contains("scalar"));
    }
}
