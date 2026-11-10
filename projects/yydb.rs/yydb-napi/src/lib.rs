//! Thin Node-API binding — atomic database operations over the [`database`] facade.
//!
//! The binding exposes one handle per open file plus single-purpose methods.
//! User-facing commands (`init`, `info`, `serve`) are composed in TypeScript so
//! this crate never grows CLI aggregate helpers.
//!
//! Every `database::types::Error` surface carries the full diagnostic contract: the JS
//! `Error.message` is a compact JSON object produced by
//! [`database::types::Error::to_json`]. Hosts that need only the human-readable text must
//! parse the JSON and pick the `"message"` key; branching logic should prefer
//! the `"code"` key.

#![deny(missing_docs)]

use std::{
    collections::BTreeMap,
    ffi::c_void,
    sync::{
        Arc, Mutex,
        atomic::{AtomicPtr, Ordering},
    },
};

use napi::bindgen_prelude::{Buffer, Env, FunctionRef, Result};
use napi_derive::napi;

/// Upstream diagnostic namespace produced by `database::types::Error::code`.
const UPSTREAM_CODE_PREFIX: &str = "database.";

/// Host-facing diagnostic namespace for the same errors.
const HOST_CODE_PREFIX: &str = "yydb.";

/// Rewrites the upstream code namespace to the host contract spelling.
fn retag_error_code(mut json: serde_json::Value) -> serde_json::Value {
    if let Some(serde_json::Value::String(code)) = json.get_mut("code") {
        if let Some(tail) = code.strip_prefix(UPSTREAM_CODE_PREFIX) {
            *code = format!("{HOST_CODE_PREFIX}{tail}");
        }
    }
    json
}

fn reason(error: database::types::Error) -> napi::Error {
    let json = serde_json::to_string(&retag_error_code(error.to_json())).unwrap_or_else(|_| error.to_string());
    napi::Error::from_reason(json)
}

// ─── Prepared statement registry ──────────────────────────────────────────

thread_local! {
    /// Statement registry — one entry per `Database.prepare()` call in this
    /// thread. Keyed by `statement_id` (FNV-1a fingerprint prefix); holding
    /// the full `PreparedStatement` with lowered op tree for execution.
    static STMT_REG: std::cell::RefCell<BTreeMap<String, database::query::local::PreparedStatement>> =
        const { std::cell::RefCell::new(BTreeMap::new()) };
}

fn register_stmt(stmt: database::query::local::PreparedStatement) -> String {
    let id = stmt.statement_id.clone();
    STMT_REG.with(|r| {
        r.borrow_mut().insert(id.clone(), stmt);
    });
    id
}

fn take_stmt(id: &str) -> Option<database::query::local::PreparedStatement> {
    STMT_REG.with(|r| r.borrow_mut().remove(id))
}

fn with_stmt<F, R>(id: &str, f: F) -> Result<R>
where
    F: FnOnce(&database::query::local::PreparedStatement) -> Result<R>,
{
    STMT_REG.with(|r| {
        let borrow = r.borrow();
        let stmt = borrow
            .get(id)
            .ok_or_else(|| napi::Error::from_reason(format!("statement `{id}` has been closed or never prepared")))?;
        f(stmt)
    })
}

fn parse_params_json(json: &str) -> Result<BTreeMap<String, database::types::Value>> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| napi::Error::from_reason(format!("invalid parameters JSON: {e}")))?;
    let obj =
        value.as_object().ok_or_else(|| napi::Error::from_reason("parameters must be a JSON object {name: value, ...}"))?;
    let mut map = BTreeMap::new();
    for (key, val) in obj {
        let yydb_val = json_to_value(val.clone())?;
        map.insert(key.clone(), yydb_val);
    }
    Ok(map)
}

/// Embedded `yydb` engine version string.
#[napi]
pub fn version() -> String {
    database::execution::engine::version().to_string()
}

/// Run the YY wire serve loop on `bind` for the database at `db_path`.
///
/// This call blocks until the listener stops. Only a dedicated process such as
/// the `yydb serve` CLI should invoke it.
#[napi]
pub fn serve(db_path: String, bind: String, insecure_bind: Option<bool>) -> Result<()> {
    database::server::yydb::run_serve(std::path::Path::new(&db_path), &bind, insecure_bind.unwrap_or(false))
        .map_err(|error| napi::Error::from_reason(error.to_string()))
}

// ─── Host runtime adapter ─────────────────────────────────────────────────

/// Node-side [`database::udf::HostRuntimeAdapter`] that calls back into the JS invoker
/// installed through [`Database::install_host_invoker`].
///
/// The engine only invokes a host micro synchronously from inside a `#[napi]`
/// call on the thread that installed the invoker, so the raw `napi_env`
/// captured at install time is still live when [`Self::invoke_scalar`] borrows
/// the persisted [`FunctionRef`] back into a callable scope.
struct NapiHostAdapter {
    /// Raw `napi_env` captured when the JS invoker was installed; null until then.
    env: AtomicPtr<c_void>,
    /// One persistent JS callback per host session id.
    invokers: Mutex<BTreeMap<u64, Arc<FunctionRef<String, String>>>>,
}

impl NapiHostAdapter {
    fn new() -> Self {
        Self { env: AtomicPtr::new(std::ptr::null_mut()), invokers: Mutex::new(BTreeMap::new()) }
    }
}

impl database::udf::HostRuntimeAdapter for NapiHostAdapter {
    fn invoke_scalar(
        &self,
        handle: &database::udf::HostFunctionHandle,
        args: &[database::udf::UdfValue],
    ) -> database::udf::Result<database::udf::UdfValue> {
        let invoker = self
            .invokers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&handle.host_id)
            .cloned()
            .ok_or(database::udf::UdfError::HostDisconnected)?;
        let env_raw = self.env.load(Ordering::Acquire);
        if env_raw.is_null() {
            return Err(database::udf::UdfError::HostDisconnected);
        }
        let env = Env::from_raw(env_raw as napi::sys::napi_env);
        let function = invoker.borrow_back(&env).map_err(|_| database::udf::UdfError::HostDisconnected)?;
        let payload = serde_json::json!({
            "hostId": handle.host_id,
            "handleVersion": handle.version,
            "functionId": handle.function_id,
            "args": args.iter().map(udf_value_to_json).collect::<Vec<_>>(),
        });
        let payload_json = serde_json::to_string(&payload).map_err(|error| database::udf::UdfError::ExecutionFailed {
            message: format!("encode host invoke payload: {error}"),
        })?;
        let result_json = function
            .call(payload_json)
            .map_err(|error| database::udf::UdfError::ExecutionFailed { message: error.to_string() })?;
        let result: serde_json::Value =
            serde_json::from_str(&result_json).map_err(|_| database::udf::UdfError::InvalidReturnValue)?;
        json_to_udf_value(result)
    }

    fn invoke_batch(
        &self,
        _handle: &database::udf::HostFunctionHandle,
        _batches: &[Vec<database::udf::UdfValue>],
    ) -> database::udf::Result<Vec<database::udf::UdfValue>> {
        // Phase-1 host micros are scalar-only; the engine scalar path routes
        // exclusively through [`Self::invoke_scalar`].
        Err(database::udf::UdfError::UnsupportedType)
    }
}

fn udf_value_to_json(value: &database::udf::UdfValue) -> serde_json::Value {
    match value {
        database::udf::UdfValue::Null => serde_json::Value::Null,
        database::udf::UdfValue::Bool(b) => serde_json::Value::Bool(*b),
        database::udf::UdfValue::I64(i) => serde_json::Value::Number((*i).into()),
        database::udf::UdfValue::Text(s) => serde_json::Value::String(s.clone()),
    }
}

fn json_to_udf_value(value: serde_json::Value) -> database::udf::Result<database::udf::UdfValue> {
    match value {
        serde_json::Value::Null => Ok(database::udf::UdfValue::Null),
        serde_json::Value::Bool(b) => Ok(database::udf::UdfValue::Bool(b)),
        serde_json::Value::Number(n) => {
            n.as_i64().map(database::udf::UdfValue::I64).ok_or(database::udf::UdfError::InvalidReturnValue)
        }
        serde_json::Value::String(s) => Ok(database::udf::UdfValue::Text(s)),
        _ => Err(database::udf::UdfError::InvalidReturnValue),
    }
}

/// Handle to one open `.yydb` file.
#[napi]
pub struct Database {
    /// The connection is released on [`Database::close`] so the host can drop
    /// the file and writer lock without waiting for garbage collection.
    connection: Mutex<Option<Arc<database::execution::engine::Connection>>>,
    host_adapter: Arc<NapiHostAdapter>,
}

impl Database {
    fn handle(&self) -> Result<Arc<database::execution::engine::Connection>> {
        self.connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .ok_or_else(|| napi::Error::from_reason("database handle has been closed"))
    }
}

#[napi]
impl Database {
    /// Open (create when missing) the database at `db_path`.
    #[napi(factory)]
    pub fn open(db_path: String) -> Result<Self> {
        let connection = database::execution::engine::Connection::open(&db_path).map_err(reason)?;
        let host_adapter = Arc::new(NapiHostAdapter::new());
        let adapter: Arc<dyn database::udf::HostRuntimeAdapter> = host_adapter.clone();
        connection.set_host_adapter(adapter);
        Ok(Self { connection: Mutex::new(Some(Arc::new(connection))), host_adapter })
    }

    /// Open a verified backup copy: `.yydb` checks every page, `.yydx` checks
    /// the object tree closure and file digests.
    ///
    /// The backup opens in place, this factory does not overwrite a live
    /// database. Copy the backup to its destination, then open that path.
    #[napi(factory)]
    pub fn restore(backup: String) -> Result<Self> {
        let connection = if backup.ends_with(".yydx") {
            database::execution::engine::Connection::restore_yydx(&backup).map_err(reason)?
        }
        else {
            database::execution::engine::Connection::restore_yydb(&backup).map_err(reason)?
        };
        let host_adapter = Arc::new(NapiHostAdapter::new());
        let adapter: Arc<dyn database::udf::HostRuntimeAdapter> = host_adapter.clone();
        connection.set_host_adapter(adapter);
        Ok(Self { connection: Mutex::new(Some(Arc::new(connection))), host_adapter })
    }

    /// Release the file handle and its writer lock.
    ///
    /// Returns `true` when this call released the handle. Later calls on the
    /// same JS object fail with a closed-handle error, and a second `close()`
    /// returns `false`.
    #[napi]
    pub fn close(&self) -> bool {
        self.connection.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take().is_some()
    }

    /// Absolute path of the database file.
    #[napi]
    pub fn path(&self) -> Option<String> {
        let handle = self.handle().ok()?;
        handle.path().map(|path| path.display().to_string())
    }

    /// Path of the `-wal` sidecar for file-backed databases.
    #[napi]
    pub fn wal_path(&self) -> Option<String> {
        let handle = self.handle().ok()?;
        handle.wal_path().map(|path| path.display().to_string())
    }

    /// Path of the `-shm` sidecar for file-backed databases.
    #[napi]
    pub fn shm_path(&self) -> Option<String> {
        let handle = self.handle().ok()?;
        handle.shm_path().map(|path| path.display().to_string())
    }

    /// Published schema version, or `null` before the first schema.
    #[napi]
    pub fn schema_version(&self) -> Result<Option<u32>> {
        Ok(self.handle()?.schema().map_err(reason)?.map(|schema| schema.version))
    }

    /// Published VOS schema document, or `null` before the first schema.
    #[napi]
    pub fn schema_document(&self) -> Result<Option<String>> {
        Ok(self.handle()?.schema().map_err(reason)?.map(|schema| schema.document))
    }

    /// Validate and publish a VOS schema document.
    #[napi]
    pub fn ensure_schema(&self, document: String) -> Result<()> {
        self.handle()?.ensure_schema(&document).map_err(reason)
    }

    /// Number of user records in the database.
    #[napi]
    pub fn record_count(&self) -> Result<u32> {
        let count = self.handle()?.record_count().map_err(reason)?;
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }

    /// Current journal mode (`delete` or `wal`).
    #[napi]
    pub fn journal_mode(&self) -> Result<String> {
        Ok(self.handle()?.journal_mode().as_str().to_owned())
    }

    /// Switch the journal mode.
    #[napi]
    pub fn set_journal_mode(&self, mode: String) -> Result<()> {
        let mode = database::execution::engine::JournalMode::parse(&mode).map_err(reason)?;
        self.handle()?.set_journal_mode(mode).map_err(reason)
    }

    /// Fold the WAL into the main file and truncate the sidecar.
    #[napi]
    pub fn checkpoint(&self) -> Result<()> {
        self.handle()?.checkpoint().map_err(reason)
    }

    /// Number of frames currently in the WAL.
    #[napi]
    pub fn wal_frame_count(&self) -> Result<u32> {
        self.handle()?.wal_frame_count().map_err(reason)
    }

    /// Read one record, or `null` when the key is absent.
    #[napi]
    pub fn get(&self, key: String) -> Result<Option<Buffer>> {
        Ok(self.handle()?.get(&key).map_err(reason)?.map(Buffer::from))
    }

    /// Write one record.
    #[napi]
    pub fn put(&self, key: String, value: Buffer) -> Result<()> {
        self.handle()?.put(key, value.as_ref()).map_err(reason)
    }

    /// Delete one record. Idempotent — absent keys silently succeed.
    #[napi]
    pub fn delete(&self, key: String) -> Result<()> {
        self.handle()?.write_batch(&[(key, None)]).map_err(reason)
    }

    /// Scan keys in lexical order with `prefix` (use `""` for all keys).
    /// `after` is an exclusive continuation cursor (pass `null` for the first page).
    /// Returns a JSON array string: `[{ "key": "...", "value": [...] }, ...]`.
    #[napi]
    pub fn scan_prefix(&self, prefix: String, after: Option<String>, limit: u32) -> Result<String> {
        let rows = self.handle()?.scan_prefix(&prefix, after.as_deref(), limit as usize).map_err(reason)?;
        let json_rows: Vec<serde_json::Value> = rows
            .into_iter()
            .map(|(k, v)| {
                serde_json::json!({
                    "key": k,
                    "value": v,
                })
            })
            .collect();
        serde_json::to_string(&json_rows).map_err(|e| napi::Error::from_reason(format!("encode scan rows: {e}")))
    }

    /// Atomically write or delete many keys.
    /// `changes_json` is `[{ "key": "...", "value": bytes | null }, ...]`.
    #[napi]
    pub fn write_batch(&self, changes_json: String) -> Result<()> {
        let changes: Vec<serde_json::Value> =
            serde_json::from_str(&changes_json).map_err(|e| napi::Error::from_reason(format!("invalid batch JSON: {e}")))?;
        let mut parsed: Vec<(String, Option<Vec<u8>>)> = Vec::with_capacity(changes.len());
        for entry in changes {
            let obj = entry.as_object().ok_or_else(|| napi::Error::from_reason("batch entry must be an object"))?;
            let key = obj
                .get("key")
                .and_then(|v| v.as_str())
                .ok_or_else(|| napi::Error::from_reason("batch entry missing string `key`"))?
                .to_owned();
            let value = match obj.get("value") {
                Some(serde_json::Value::Null) | None => None,
                Some(v) => {
                    let bytes: Vec<u8> = v
                        .as_array()
                        .and_then(|arr| arr.iter().map(|x| x.as_i64().and_then(|n| u8::try_from(n).ok())).collect())
                        .ok_or_else(|| napi::Error::from_reason("batch `value` must be null or a byte array"))?;
                    Some(bytes)
                }
            };
            parsed.push((key, value));
        }
        self.handle()?.write_batch(&parsed).map_err(reason)
    }

    /// Compare-and-swap: replace or delete `key` only if it currently matches `expected`.
    /// Both are JSON arrays of bytes (from `Buffer`) or `null`.
    /// Returns `true` on success, `false` if the current value did not match.
    #[napi]
    pub fn compare_exchange(&self, key: String, expected: Option<Buffer>, replacement: Option<Buffer>) -> Result<bool> {
        self.handle()?.compare_exchange(&key, expected.as_deref(), replacement.as_deref()).map_err(reason)
    }

    /// Begin an explicit transaction. Must be balanced with `commit` or `rollback`.
    /// The Node facade wraps these with a callback-style `transaction()` helper.
    #[napi]
    pub fn begin(&self) -> Result<()> {
        self.handle()?.begin().map_err(reason)
    }

    /// Commit the current transaction.
    #[napi]
    pub fn commit(&self) -> Result<()> {
        self.handle()?.commit().map_err(reason)
    }

    /// Roll back the current transaction.
    #[napi]
    pub fn rollback(&self) -> Result<()> {
        self.handle()?.rollback().map_err(reason)
    }

    /// Create a named savepoint inside the open transaction.
    #[napi]
    pub fn savepoint(&self, name: String) -> Result<()> {
        self.handle()?.savepoint(&name).map_err(reason)
    }

    /// Roll the open transaction back to a previously created savepoint,
    /// discarding writes made after it and every savepoint nested above.
    #[napi]
    pub fn rollback_to_savepoint(&self, name: String) -> Result<()> {
        self.handle()?.rollback_to_savepoint(&name).map_err(reason)
    }

    /// Release a named savepoint so it can no longer be rolled back to.
    #[napi]
    pub fn release_savepoint(&self, name: String) -> Result<()> {
        self.handle()?.release_savepoint(&name).map_err(reason)
    }

    /// Execute a VOS read pipeline and return the resulting rows.
    #[napi]
    pub fn query_vos(&self, source: String) -> Result<String> {
        let rows = self.handle()?.query_vos(&source).map_err(reason)?;
        let mapped: Vec<serde_json::Value> = rows
            .into_iter()
            .map(|row| {
                let mut map = serde_json::Map::new();
                for (k, v) in row {
                    map.insert(k, value_to_json(v));
                }
                serde_json::Value::Object(map)
            })
            .collect();
        serde_json::to_string(&mapped).map_err(|e| napi::Error::from_reason(format!("encode rows: {e}")))
    }

    /// Execute a VOS write program (INSERT / schema macros).
    ///
    /// `params_json` is `{ "paramName": <value>, ... }` or `null` when the
    /// program takes no parameters. Values may use the tagged scalar form
    /// (`{"$yydb":"bytes","hex":...}` and the other exact scalars) so octets and
    /// exact calendar values cross the host boundary unmodified. The
    /// `"$database"` spelling is accepted as well, with `$database` taking
    /// precedence when both keys are present.
    #[napi]
    pub fn execute_vos_write(&self, source: String, params_json: Option<String>) -> Result<()> {
        let named = match params_json {
            Some(json) if !json.trim().is_empty() && json.trim() != "null" => parse_params_json(&json)?,
            _ => BTreeMap::new(),
        };
        self.handle()?.execute_vos_write_with_params(&source, &named).map_err(reason)
    }

    /// Prepare a VOS read pipeline into a reusable query plan.
    ///
    /// The returned [`Statement`] holds the `statement_id`; the lowered op
    /// tree, parameter descriptors and source fingerprint stay in a
    /// thread-local registry keyed by that id. Callers re-use the plan via
    /// [`Self::query_prepared`] or drop it explicitly with
    /// [`Statement::close`].
    ///
    /// Requires no schema binding — plan building is a pure parse + lower pass.
    /// A write program has no reusable read plan, so it is refused here and
    /// must go through [`Self::execute_vos_write`].
    #[napi]
    pub fn prepare(&self, source: String) -> Result<Statement> {
        let stmt = self.handle()?.prepare_vos(&source).map_err(reason)?;
        if stmt.query_kind != database::query::local::QueryKind::Read {
            return Err(reason(database::types::Error::Schema {
                message: "Phase 1 only executes pipelines ending in `.collect()`".into(),
            }));
        }
        let id = register_stmt(stmt);
        Ok(Statement { statement_id: id })
    }

    /// Execute a previously prepared statement with optional named parameter
    /// bindings.
    ///
    /// `params_json` is `{ "paramName": <value>, ... }` or `null` when the
    /// statement takes no parameters. Missing / extraneous / mistyped
    /// parameters surface as `yydb.query.bind`.
    #[napi]
    pub fn query_prepared(&self, statement: &Statement, params_json: Option<String>) -> Result<String> {
        let named = match params_json {
            Some(json) if !json.trim().is_empty() && json.trim() != "null" => parse_params_json(&json)?,
            _ => BTreeMap::new(),
        };
        let rows = with_stmt(&statement.statement_id, |stmt| self.handle()?.query_prepared(stmt, &named).map_err(reason))?;
        let mapped: Vec<serde_json::Value> = rows
            .into_iter()
            .map(|row| {
                let mut map = serde_json::Map::new();
                for (k, v) in row {
                    map.insert(k, value_to_json(v));
                }
                serde_json::Value::Object(map)
            })
            .collect();
        serde_json::to_string(&mapped).map_err(|e| napi::Error::from_reason(format!("encode rows: {e}")))
    }

    /// Publish a new schema version with an optional rename map.
    ///
    /// `renames_json` is `[{ "fromType": "...", "toType": "..." }, { "type": "...",
    /// "fromField": "...", "toField": "..." }]` or `null` when the migration drops
    /// no fields. Entries with `fromType`/`toType` update the type name; entries
    /// with `type`/`fromField`/`toField` update a specific field.
    #[napi]
    pub fn migrate_schema(&self, document: String, renames_json: Option<String>) -> Result<()> {
        let mut renames = database::execution::engine::catalog::RenameMap::default();
        if let Some(json) = renames_json {
            let arr: Vec<serde_json::Value> =
                serde_json::from_str(&json).map_err(|e| napi::Error::from_reason(format!("invalid renames JSON: {e}")))?;
            for entry in arr {
                let obj = entry.as_object().ok_or_else(|| napi::Error::from_reason("rename entry must be an object"))?;
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
                    return Err(napi::Error::from_reason(
                        "rename entry must have {fromType,toType} or {type,fromField,toField}",
                    ));
                }
            }
        }
        self.handle()?.migrate_schema(&document, &renames).map_err(reason)
    }

    /// Checkpoint then copy a self-contained `.yydb` main file to `dest`.
    ///
    /// WAL mode connections are checkpointed so the backup needs no `-wal` /
    /// `-shm` sidecars. In-memory connections return an error because there is
    /// nothing to copy.
    #[napi]
    pub fn backup(&self, dest: String) -> Result<()> {
        self.handle()?.backup_to(&dest).map_err(reason)
    }

    /// Copy the main file and any present `-wal` / `-shm` sidecars without
    /// checkpoint. Opening `dest` replays the copied WAL tail against the main
    /// image.
    #[napi]
    pub fn backup_with_wal(&self, dest: String) -> Result<()> {
        self.handle()?.backup_with_wal_to(&dest).map_err(reason)
    }

    /// Run read-only consistency probes. Returns the `DoctorReport` as JSON.
    #[napi]
    pub fn doctor(&self) -> Result<String> {
        let report = self.handle()?.doctor().map_err(reason)?;
        serde_json::to_string(&report).map_err(|e| napi::Error::from_reason(format!("serialize doctor report: {e}")))
    }

    /// Fold WAL into the main file and reclaim safe orphan blobs (`.yydx` layouts only).
    /// Returns the `CompactReport` as JSON.
    #[napi]
    pub fn compact_yydx(&self) -> Result<String> {
        let report = self.handle()?.compact_yydx().map_err(reason)?;
        serde_json::to_string(&report).map_err(|e| napi::Error::from_reason(format!("serialize compact report: {e}")))
    }

    /// Install a session-local host micro definition.
    ///
    /// `args` is an array of UDF type tags (`null`, `bool`, `i64`, `text`).
    #[napi]
    pub fn register_micro(
        &self,
        name: String,
        version: u32,
        args: Vec<String>,
        returns: String,
        host_id: i64,
        function_id: String,
        handle_version: u32,
        fingerprint: Buffer,
    ) -> Result<()> {
        let arg_types = args.iter().map(|t| parse_udf_type(t)).collect::<std::result::Result<Vec<_>, _>>()?;
        let returns_type = parse_udf_type(&returns)?;
        let fp_bytes: [u8; 32] =
            fingerprint.as_ref().try_into().map_err(|_| napi::Error::from_reason("fingerprint must be exactly 32 bytes"))?;
        let definition = database::udf::HostMicroDefinition::from_scalar(
            &name,
            version,
            arg_types,
            returns_type,
            host_id as u64,
            function_id,
            handle_version,
            fp_bytes,
        )
        .map_err(|e| napi::Error::from_reason(e.to_string()))?;
        self.handle()?.register_host_micro(definition).map_err(reason)
    }

    /// Publish a logical `file` value: chunks first, then the manifest object.
    ///
    /// Both layouts support it: `.yydx` keeps the object tree in
    /// `<stem>-objects/`, and a single-file `.yydb` embeds the same objects as
    /// records in its main image. Embedding large payloads inflates the single
    /// file and slows down snapshots and backups, so a `.yydx` layout is
    /// recommended for object-heavy workloads.
    /// Returns the descriptor as a tagged JSON object with the host-contract
    /// shape `{"$yydb":"file",…}` that can be round-tripped through VOS write
    /// parameters or `read_file_window`.
    #[napi]
    pub fn put_file(
        &self,
        payload: Buffer,
        chunk_size: u32,
        name: Option<String>,
        media_type: Option<String>,
    ) -> Result<String> {
        use std::io::Cursor;
        let chunk_size =
            usize::try_from(chunk_size).map_err(|_| napi::Error::from_reason("chunk_size exceeds platform pointer width"))?;
        let descriptor =
            self.handle()?.put_file(Cursor::new(payload.as_ref()), chunk_size, name, media_type).map_err(reason)?;
        let json = database::types::scalar::tagged_value_json(&database::types::Value::File(descriptor))
            .expect("`Value::File` always has a tagged form");
        let json = retag_database_to_host(json);
        serde_json::to_string(&json).map_err(|e| napi::Error::from_reason(format!("encode file descriptor: {e}")))
    }

    /// Read one VOS window of a committed `file` value.
    ///
    /// `descriptor_json` must be a tagged JSON `{"$yydb":"file",…}` produced by
    /// [`Self::put_file`] or queried from a VOS `file` column. The legacy
    /// `"$database"` spelling is accepted too.
    /// Every touched chunk is verified against the manifest.
    #[napi]
    pub fn read_file_window(&self, descriptor_json: String, offset: i64, length: i64) -> Result<Buffer> {
        let descriptor = parse_file_descriptor(&descriptor_json)?;
        let offset = u64::try_from(offset).map_err(|_| napi::Error::from_reason("offset must be non-negative"))?;
        let length = u64::try_from(length).map_err(|_| napi::Error::from_reason("length must be non-negative"))?;
        let window = vos::types::FileReadWindow::new(offset, length);
        let octets = self.handle()?.read_file_window(&descriptor, window).map_err(reason)?;
        Ok(Buffer::from(octets))
    }

    /// Read the complete octets of a committed `file` value and verify its
    /// content digest.
    #[napi]
    pub fn read_file(&self, descriptor_json: String) -> Result<Buffer> {
        let descriptor = parse_file_descriptor(&descriptor_json)?;
        let octets = self.handle()?.read_file(&descriptor).map_err(reason)?;
        Ok(Buffer::from(octets))
    }

    /// Invoke a registered scalar UDF.
    ///
    /// `args_json` is a JSON array of scalar values. Returns result as JSON.
    #[napi]
    pub fn call_scalar(&self, name: String, version: u32, args_json: String) -> Result<String> {
        let args: Vec<serde_json::Value> =
            serde_json::from_str(&args_json).map_err(|e| napi::Error::from_reason(format!("invalid args JSON: {e}")))?;
        let values: Vec<database::types::Value> =
            args.into_iter().map(json_to_value).collect::<std::result::Result<Vec<_>, _>>()?;
        let result = self.handle()?.call_scalar_version(&name, version, &values).map_err(reason)?;
        serde_json::to_string(&value_to_json(result)).map_err(|e| napi::Error::from_reason(format!("encode result: {e}")))
    }

    /// Install the JS callback that executes host micro invocations for `host_id`.
    ///
    /// `invoker` receives one JSON payload string
    /// `{"hostId":…,"handleVersion":…,"functionId":…,"args":[…]}` and must
    /// return the scalar result as a JSON string. Until an invoker is
    /// installed for a host id, registered host micros fail as
    /// `yydb.udf.failed` with a host-disconnected message.
    #[napi]
    pub fn install_host_invoker(&self, env: Env, host_id: i64, invoker: FunctionRef<String, String>) -> Result<()> {
        let host_id = u64::try_from(host_id).map_err(|_| napi::Error::from_reason("host_id must be non-negative"))?;
        self.host_adapter.env.store(env.raw() as *mut c_void, Ordering::Release);
        self.host_adapter.invokers.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(host_id, Arc::new(invoker));
        Ok(())
    }
}

/// Handle to a remote `yydb serve` endpoint (Rust TCP client).
///
/// Provides the same atomic operations as the embedded [`Database`], but over
/// the wire. Node hosts that need both local and remote access can hold one of
/// each; this crate never merges them into a single facade.
#[napi]
pub struct Client {
    client: database::client::yy::Client,
    closed: bool,
}

#[napi]
impl Client {
    /// Connect to a serve endpoint such as `127.0.0.1:7700` or `ws://host:port/wire`.
    ///
    /// Strips optional `tcp://` / `http://` / `ws://` prefixes. Performs the
    /// wire Hello handshake before returning.
    #[napi(factory)]
    pub fn connect(endpoint: String) -> Result<Self> {
        Ok(Self { client: database::client::yy::Client::connect(endpoint).map_err(reason)?, closed: false })
    }

    /// Configured endpoint string (`host:port`).
    #[napi]
    pub fn endpoint(&self) -> String {
        self.client.endpoint().to_owned()
    }

    /// Server version string from the latest Hello handshake.
    #[napi]
    pub fn server_version(&self) -> Result<String> {
        self.client.server_version().map_err(reason)
    }

    /// Fetch remote info text (`path=…\n…`).
    #[napi]
    pub fn info(&self) -> Result<String> {
        self.client.info().map_err(reason)
    }

    /// Published schema version, or `null` before the first schema.
    #[napi]
    pub fn schema_version(&self) -> Result<Option<u32>> {
        Ok(self.client.schema().map_err(reason)?.map(|schema| schema.version))
    }

    /// Published VOS schema document, or `null` before the first schema.
    #[napi]
    pub fn schema_document(&self) -> Result<Option<String>> {
        Ok(self.client.schema().map_err(reason)?.map(|schema| schema.document))
    }

    /// Validate and publish a VOS schema document on the remote engine.
    #[napi]
    pub fn ensure_schema(&self, document: String) -> Result<()> {
        self.client.ensure_schema(&document).map_err(reason)
    }

    /// Read one remote record, or `null` when the key is absent.
    #[napi]
    pub fn get(&self, key: String) -> Result<Option<Buffer>> {
        Ok(self.client.get(&key).map_err(reason)?.map(Buffer::from))
    }

    /// Write one remote record.
    #[napi]
    pub fn put(&self, key: String, value: Buffer) -> Result<()> {
        self.client.put(&key, value.as_ref()).map_err(reason)
    }

    /// Remote client does not expose record count; use info() for diagnostics.
    #[napi]
    pub fn record_count(&self) -> Result<u32> {
        Err(napi::Error::from_reason("record_count is not available on the remote client; use info() for diagnostics"))
    }

    /// Install a session-local host micro definition on the remote engine.
    ///
    /// `args` is an array of UDF type tags (`null`, `bool`, `i64`, `text`).
    #[napi]
    pub fn register_micro(
        &self,
        name: String,
        version: u32,
        args: Vec<String>,
        returns: String,
        host_id: i64,
        function_id: String,
        handle_version: u32,
        fingerprint: Buffer,
    ) -> Result<()> {
        if self.closed {
            return Err(napi::Error::from_reason("client is closed"));
        }
        let arg_types = args.iter().map(|t| parse_udf_type(t)).collect::<std::result::Result<Vec<_>, _>>()?;
        let returns_type = parse_udf_type(&returns)?;
        let fp_bytes: [u8; 32] =
            fingerprint.as_ref().try_into().map_err(|_| napi::Error::from_reason("fingerprint must be exactly 32 bytes"))?;
        let definition = database::udf::HostMicroDefinition::from_scalar(
            &name,
            version,
            arg_types,
            returns_type,
            host_id as u64,
            function_id,
            handle_version,
            fp_bytes,
        )
        .map_err(|e| napi::Error::from_reason(e.to_string()))?;
        self.client.register_host_micro(&definition).map_err(reason)
    }

    /// Invoke a registered scalar UDF on the remote engine.
    ///
    /// `args_json` is a JSON array of scalar values (`null`, `bool`, `number`,
    /// `string`). Returns the scalar result as a JSON string.
    #[napi]
    pub fn call_scalar(&self, name: String, version: u32, args_json: String) -> Result<String> {
        if self.closed {
            return Err(napi::Error::from_reason("client is closed"));
        }
        let args: Vec<serde_json::Value> =
            serde_json::from_str(&args_json).map_err(|e| napi::Error::from_reason(format!("invalid args JSON: {e}")))?;
        let values: Vec<database::types::Value> =
            args.into_iter().map(json_to_value).collect::<std::result::Result<Vec<_>, _>>()?;
        let result = self.client.call_scalar_version(&name, version, &values).map_err(reason)?;
        serde_json::to_string(&value_to_json(result)).map_err(|e| napi::Error::from_reason(format!("encode result: {e}")))
    }

    /// Close the remote client. Idempotent — further calls fail.
    #[napi]
    pub fn close(&mut self) {
        self.closed = true;
    }
}

/// napi 边界 DTO：单个参数描述符的 JSON 形态。
///
/// 核心类型 [`database::query::local::ParameterDescriptor`] 保持 Rust 命名，
/// JS 侧的 camelCase 拼写只由本 DTO 负责，便于核心层后续去掉 serde 重命名。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ParameterDescriptorJson<'a> {
    /// 参数名（不含前导 `$`）。
    name: &'a str,
    /// 绑定值切片中的位置下标。
    index: u32,
    /// 声明的槽位类型。
    slot_ty: database::query::local::SlotTy,
    /// 参数在源码中出现的次数。
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

/// napi 边界 DTO：预编译语句元数据的 JSON 形态。
///
/// 字段集合与顺序对齐 `PreparedStatement` 的公开序列化形态，键名由此处冻结，
/// 核心层不再承担面向 JS 的重命名。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StatementInfoJson<'a> {
    /// 语句 id。
    statement_id: &'a str,
    /// 前端方言。
    dialect: database::query::local::Dialect,
    /// 查询类别。
    query_kind: database::query::local::QueryKind,
    /// 参数描述符列表。
    parameters: Vec<ParameterDescriptorJson<'a>>,
    /// 读表集合。
    read_set: &'a [String],
    /// 写表集合。
    write_set: &'a [String],
    /// 源码指纹。
    source_fingerprint: &'a str,
    /// 源码文本。
    source: &'a str,
    /// schema 指纹。
    schema_fingerprint: &'a str,
    /// schema 修订号。
    schema_revision: u64,
}

impl<'a> StatementInfoJson<'a> {
    fn new(statement: &'a database::query::local::PreparedStatement) -> Self {
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

/// Handle to a prepared VOS query plan.
///
/// Created by [`Database::prepare`]. The actual lowered op tree lives in a
/// thread-local registry keyed by [`Self::statement_id`]; this struct is a
/// lightweight id handle for the JS side. Call [`Self::close`] (or rely on
/// GC-collection after the JS side drops the handle) to release the plan.
#[napi]
pub struct Statement {
    statement_id: String,
}

#[napi]
impl Statement {
    /// The fingerprint-prefixed unique id for this statement.
    #[napi]
    pub fn statement_id(&self) -> String {
        self.statement_id.clone()
    }

    /// Parameter descriptors as JSON — the same schema as the
    /// `parameters` field on a `PreparedStatement`. Empty array when the
    /// statement takes no parameters.
    #[napi]
    pub fn parameters_json(&self) -> Result<String> {
        with_stmt(&self.statement_id, |stmt| {
            let descriptors: Vec<ParameterDescriptorJson<'_>> =
                stmt.parameters.iter().map(ParameterDescriptorJson::new).collect();
            serde_json::to_string(&descriptors).map_err(|e| napi::Error::from_reason(format!("encode parameters: {e}")))
        })
    }

    /// Full statement metadata as JSON — `statementId`, `queryKind`,
    /// `parameters`, `readSet`, `writeSet`, `sourceFingerprint` and
    /// `source`. Useful for debug logging and cache-key construction.
    #[napi]
    pub fn info(&self) -> Result<String> {
        with_stmt(&self.statement_id, |stmt| {
            serde_json::to_string(&StatementInfoJson::new(stmt))
                .map_err(|e| napi::Error::from_reason(format!("encode statement info: {e}")))
        })
    }

    /// Release this statement's plan from the thread-local registry.
    /// Subsequent calls through the handle will fail. Idempotent.
    #[napi]
    pub fn close(&self) -> bool {
        take_stmt(&self.statement_id).is_some()
    }
}

fn parse_udf_type(tag: &str) -> Result<database::udf::UdfType> {
    match tag {
        "null" => Ok(database::udf::UdfType::Null),
        "bool" => Ok(database::udf::UdfType::Bool),
        "i64" => Ok(database::udf::UdfType::I64),
        "text" | "utf8" => Ok(database::udf::UdfType::Text),
        "any" => Ok(database::udf::UdfType::Any),
        other => Err(napi::Error::from_reason(format!("unsupported UDF type: {other}"))),
    }
}

/// Tagged JSON key exposed to the JS host contract.
const HOST_TAGGED_JSON_KEY: &str = "$yydb";

/// Rewrites a host tagged object to the canonical `database` key.
///
/// `database::types::scalar::tagged_value_from_json` reads only
/// `TAGGED_JSON_KEY`; the `$yydb` spelling stays accepted as a compatibility
/// input, while `$database` wins when both keys are present.
fn retag_host_to_database(json: serde_json::Value) -> serde_json::Value {
    match json {
        serde_json::Value::Object(mut map) => {
            if !map.contains_key(database::types::scalar::TAGGED_JSON_KEY) {
                if let Some(tag) = map.remove(HOST_TAGGED_JSON_KEY) {
                    map.insert(database::types::scalar::TAGGED_JSON_KEY.to_owned(), tag);
                }
            }
            serde_json::Value::Object(map)
        }
        other => other,
    }
}

/// Rewrites the canonical tagged key to the `$yydb` spelling the JS host
/// contract requires. Only the key name changes; the value structure is kept.
fn retag_database_to_host(mut json: serde_json::Value) -> serde_json::Value {
    if let serde_json::Value::Object(map) = &mut json {
        if let Some(tag) = map.remove(database::types::scalar::TAGGED_JSON_KEY) {
            map.insert(HOST_TAGGED_JSON_KEY.to_owned(), tag);
        }
    }
    json
}

fn json_to_value(json: serde_json::Value) -> Result<database::types::Value> {
    let json = retag_host_to_database(json);
    if let Some(value) = database::types::scalar::tagged_value_from_json(&json) {
        return value.map_err(|error| napi::Error::from_reason(error.to_string()));
    }
    match json {
        serde_json::Value::Null => Ok(database::types::Value::Null),
        serde_json::Value::Bool(b) => Ok(database::types::Value::Bool(b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(database::types::Value::I64(i))
            }
            else if let Some(f) = n.as_f64() {
                Ok(database::types::Value::F64(f))
            }
            else {
                Err(napi::Error::from_reason("number out of range for i64/f64"))
            }
        }
        serde_json::Value::String(s) => Ok(database::types::Value::Text(s)),
        other => Err(napi::Error::from_reason(format!("unsupported scalar value: {other:?}"))),
    }
}

fn value_to_json(value: database::types::Value) -> serde_json::Value {
    if let Some(tagged) = database::types::scalar::tagged_value_json(&value) {
        return retag_database_to_host(tagged);
    }
    match value {
        database::types::Value::Null => serde_json::Value::Null,
        database::types::Value::Bool(b) => serde_json::Value::Bool(b),
        database::types::Value::I64(i) => serde_json::Value::Number(i.into()),
        database::types::Value::U64(u) => serde_json::Value::Number(u.into()),
        database::types::Value::F64(f) => {
            serde_json::Number::from_f64(f).map(serde_json::Value::Number).unwrap_or(serde_json::Value::Null)
        }
        database::types::Value::Text(s) => serde_json::Value::String(s),
        other => serde_json::Value::String(format!("{other:?}")),
    }
}

fn parse_file_descriptor(json: &str) -> Result<database::types::FileDescriptor> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| napi::Error::from_reason(format!("invalid file descriptor JSON: {e}")))?;
    let value = retag_host_to_database(value);
    let decoded = database::types::scalar::tagged_value_from_json(&value)
        .ok_or_else(|| napi::Error::from_reason("not a tagged value JSON"))?
        .map_err(|e| napi::Error::from_reason(e.to_string()))?;
    match decoded {
        database::types::Value::File(descriptor) => Ok(descriptor),
        other => Err(napi::Error::from_reason(format!("expected a `file` tagged value, got `{}`", other.kind()))),
    }
}

/// 基准统计的 JS 形态。字段名由本 DTO 负责 camelCase，核心 `Stats` 说 Rust。
#[napi(object)]
pub struct BenchStatsJson {
    /// 中位数毫秒（中间两值平均）。
    pub median_ms: f64,
    /// p95 毫秒（nearest-rank）。
    pub p95_ms: f64,
    /// 最小毫秒。
    pub min_ms: f64,
    /// 最大毫秒。
    pub max_ms: f64,
    /// 样本轮数。
    pub rounds: u32,
}

impl From<database::harness::Stats> for BenchStatsJson {
    fn from(stats: database::harness::Stats) -> Self {
        Self {
            median_ms: stats.median_ms,
            p95_ms: stats.p95_ms,
            min_ms: stats.min_ms,
            max_ms: stats.max_ms,
            rounds: stats.rounds,
        }
    }
}

/// 对一组毫秒样本求 median/p95/极值（中间两值 median、nearest-rank p95，golden vector 钉在 `database.harness`）。
#[napi]
pub fn harness_bench_summary(samples_ms: Vec<f64>) -> napi::Result<BenchStatsJson> {
    database::harness::summarize(&samples_ms).map(Into::into).map_err(|problem| napi::Error::from_reason(problem.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_descriptor() -> database::types::FileDescriptor {
        database::types::FileDescriptor {
            digest: "0f".repeat(32),
            octets: 17,
            name: Some("report.txt".into()),
            media_type: None,
            manifest: database::types::FileManifestRef { hash: [0x22; 32], size: 40 },
        }
    }

    #[test]
    fn file_descriptor_tagged_json_roundtrip() {
        let descriptor = sample_descriptor();
        let value = database::types::Value::File(descriptor.clone());
        let json = database::types::scalar::tagged_value_json(&value).expect("has tagged form");
        let json_str = serde_json::to_string(&json).unwrap();

        let parsed = parse_file_descriptor(&json_str).expect("descriptor must parse");
        assert_eq!(parsed, descriptor);
    }

    #[test]
    fn parse_file_descriptor_rejects_non_file_tag() {
        let json = r#"{"$yydb":"bytes","hex":"0a0b"}"#;
        assert!(parse_file_descriptor(json).is_err(), "bytes tag must not decode as file");
    }

    #[test]
    fn put_file_and_read_roundtrip_through_database() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let dir = std::env::temp_dir()
            .join(format!("yydb-napi-file-test-{}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("host-file.yydx");
        let db = Database::open(db_path.to_string_lossy().to_string()).unwrap();

        let payload = vec![1_u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17];
        let descriptor_json = db
            .put_file(Buffer::from(payload.as_slice()), 8, Some("tiny.bin".into()), None)
            .expect("put_file must succeed on .yydx layout");

        let full = db.read_file(descriptor_json.clone()).expect("read_file must succeed");
        assert_eq!(full.as_ref(), payload.as_slice());

        let window = db.read_file_window(descriptor_json, 4, 7).expect("read_file_window must succeed");
        assert_eq!(window.as_ref(), &payload[4..11]);

        db.close();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
