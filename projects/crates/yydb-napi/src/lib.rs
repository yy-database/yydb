//! Thin Node-API binding — atomic database operations over the [`yydb`] facade.
//!
//! The binding exposes one handle per open file plus single-purpose methods.
//! User-facing commands (`init`, `info`, `serve`) are composed in TypeScript so
//! this crate never grows CLI aggregate helpers.
//!
//! Every `yydb::Error` surface carries the full diagnostic contract: the JS
//! `Error.message` is a compact JSON object produced by
//! [`yydb::Error::to_json`]. Hosts that need only the human-readable text must
//! parse the JSON and pick the `"message"` key; branching logic should prefer
//! the `"code"` key.

#![deny(missing_docs)]

use napi::bindgen_prelude::{Buffer, Result};
use napi_derive::napi;

fn reason(error: yydb::Error) -> napi::Error {
    let json = serde_json::to_string(&error.to_json())
        .unwrap_or_else(|_| error.to_string());
    napi::Error::from_reason(json)
}

/// Embedded `yydb` engine version string.
#[napi]
pub fn version() -> String {
    yydb::version().to_string()
}

/// Run the YY wire serve loop on `bind` for the database at `db_path`.
///
/// This call blocks until the listener stops. Only a dedicated process such as
/// the `yydb serve` CLI should invoke it.
#[napi]
pub fn serve(db_path: String, bind: String, insecure_bind: Option<bool>) -> Result<()> {
    yydb::server::run_serve(
        std::path::Path::new(&db_path),
        &bind,
        insecure_bind.unwrap_or(false),
    )
    .map_err(|error| napi::Error::from_reason(error.to_string()))
}

/// Handle to one open `.yydb` file.
#[napi]
pub struct Database {
    connection: yydb::Connection,
}

#[napi]
impl Database {
    /// Open (create when missing) the database at `db_path`.
    #[napi(factory)]
    pub fn open(db_path: String) -> Result<Self> {
        Ok(Self {
            connection: yydb::Connection::open(&db_path).map_err(reason)?,
        })
    }

    /// Absolute path of the database file.
    #[napi]
    pub fn path(&self) -> Option<String> {
        self.connection
            .path()
            .map(|path| path.display().to_string())
    }

    /// Path of the `-wal` sidecar for file-backed databases.
    #[napi]
    pub fn wal_path(&self) -> Option<String> {
        self.connection
            .wal_path()
            .map(|path| path.display().to_string())
    }

    /// Path of the `-shm` sidecar for file-backed databases.
    #[napi]
    pub fn shm_path(&self) -> Option<String> {
        self.connection
            .shm_path()
            .map(|path| path.display().to_string())
    }

    /// Published schema version, or `null` before the first schema.
    #[napi]
    pub fn schema_version(&self) -> Result<Option<u32>> {
        Ok(self
            .connection
            .schema()
            .map_err(reason)?
            .map(|schema| schema.version))
    }

    /// Published VOS schema document, or `null` before the first schema.
    #[napi]
    pub fn schema_document(&self) -> Result<Option<String>> {
        Ok(self
            .connection
            .schema()
            .map_err(reason)?
            .map(|schema| schema.document))
    }

    /// Validate and publish a VOS schema document.
    #[napi]
    pub fn ensure_schema(&self, document: String) -> Result<()> {
        self.connection.ensure_schema(&document).map_err(reason)
    }

    /// Number of user records in the database.
    #[napi]
    pub fn record_count(&self) -> Result<u32> {
        let count = self.connection.record_count().map_err(reason)?;
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }

    /// Current journal mode (`delete` or `wal`).
    #[napi]
    pub fn journal_mode(&self) -> String {
        self.connection.journal_mode().as_str().to_owned()
    }

    /// Switch the journal mode.
    #[napi]
    pub fn set_journal_mode(&self, mode: String) -> Result<()> {
        let mode = yydb::JournalMode::parse(&mode).map_err(reason)?;
        self.connection.set_journal_mode(mode).map_err(reason)
    }

    /// Fold the WAL into the main file and truncate the sidecar.
    #[napi]
    pub fn checkpoint(&self) -> Result<()> {
        self.connection.checkpoint().map_err(reason)
    }

    /// Number of frames currently in the WAL.
    #[napi]
    pub fn wal_frame_count(&self) -> Result<u32> {
        self.connection.wal_frame_count().map_err(reason)
    }

    /// Read one record, or `null` when the key is absent.
    #[napi]
    pub fn get(&self, key: String) -> Result<Option<Buffer>> {
        Ok(self.connection.get(&key).map_err(reason)?.map(Buffer::from))
    }

    /// Write one record.
    #[napi]
    pub fn put(&self, key: String, value: Buffer) -> Result<()> {
        self.connection.put(key, value.as_ref()).map_err(reason)
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
        let arg_types = args
            .iter()
            .map(|t| parse_udf_type(t))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let returns_type = parse_udf_type(&returns)?;
        let fp_bytes: [u8; 32] = fingerprint.as_ref().try_into().map_err(|_| {
            napi::Error::from_reason("fingerprint must be exactly 32 bytes")
        })?;
        let definition = yydb::HostMicroDefinition::from_scalar(
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
        self.connection.register_host_micro(definition).map_err(reason)
    }

    /// Invoke a registered scalar UDF.
    ///
    /// `args_json` is a JSON array of scalar values. Returns result as JSON.
    #[napi]
    pub fn call_scalar(
        &self,
        name: String,
        version: u32,
        args_json: String,
    ) -> Result<String> {
        let args: Vec<serde_json::Value> = serde_json::from_str(&args_json)
            .map_err(|e| napi::Error::from_reason(format!("invalid args JSON: {e}")))?;
        let values: Vec<yydb::Value> = args
            .into_iter()
            .map(json_to_value)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let result = self
            .connection
            .call_scalar_version(&name, version, &values)
            .map_err(reason)?;
        serde_json::to_string(&value_to_json(result))
            .map_err(|e| napi::Error::from_reason(format!("encode result: {e}")))
    }
}

/// Handle to a remote `yydb serve` endpoint (Rust TCP client).
///
/// Provides the same atomic operations as the embedded [`Database`], but over
/// the wire. Node hosts that need both local and remote access can hold one of
/// each; this crate never merges them into a single facade.
#[napi]
pub struct Client {
    client: yydb::client::Client,
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
        Ok(Self {
            client: yydb::client::Client::connect(endpoint).map_err(reason)?,
            closed: false,
        })
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
        Ok(self
            .client
            .schema()
            .map_err(reason)?
            .map(|schema| schema.version))
    }

    /// Published VOS schema document, or `null` before the first schema.
    #[napi]
    pub fn schema_document(&self) -> Result<Option<String>> {
        Ok(self
            .client
            .schema()
            .map_err(reason)?
            .map(|schema| schema.document))
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
        Err(napi::Error::from_reason(
            "record_count is not available on the remote client; use info() for diagnostics",
        ))
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
        let arg_types = args
            .iter()
            .map(|t| parse_udf_type(t))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let returns_type = parse_udf_type(&returns)?;
        let fp_bytes: [u8; 32] = fingerprint.as_ref().try_into().map_err(|_| {
            napi::Error::from_reason("fingerprint must be exactly 32 bytes")
        })?;
        let definition = yydb::HostMicroDefinition::from_scalar(
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
    pub fn call_scalar(
        &self,
        name: String,
        version: u32,
        args_json: String,
    ) -> Result<String> {
        if self.closed {
            return Err(napi::Error::from_reason("client is closed"));
        }
        let args: Vec<serde_json::Value> = serde_json::from_str(&args_json)
            .map_err(|e| napi::Error::from_reason(format!("invalid args JSON: {e}")))?;
        let values: Vec<yydb::Value> = args
            .into_iter()
            .map(json_to_value)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let result = self
            .client
            .call_scalar_version(&name, version, &values)
            .map_err(reason)?;
        serde_json::to_string(&value_to_json(result))
            .map_err(|e| napi::Error::from_reason(format!("encode result: {e}")))
    }

    /// Close the remote client. Idempotent — further calls fail.
    #[napi]
    pub fn close(&mut self) {
        self.closed = true;
    }
}

fn parse_udf_type(tag: &str) -> Result<yydb::UdfType> {
    match tag {
        "null" => Ok(yydb::UdfType::Null),
        "bool" => Ok(yydb::UdfType::Bool),
        "i64" => Ok(yydb::UdfType::I64),
        "text" | "utf8" => Ok(yydb::UdfType::Text),
        "any" => Ok(yydb::UdfType::Any),
        other => Err(napi::Error::from_reason(format!("unsupported UDF type: {other}"))),
    }
}

fn json_to_value(json: serde_json::Value) -> Result<yydb::Value> {
    match json {
        serde_json::Value::Null => Ok(yydb::Value::Null),
        serde_json::Value::Bool(b) => Ok(yydb::Value::Bool(b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(yydb::Value::I64(i))
            } else if let Some(f) = n.as_f64() {
                Ok(yydb::Value::F64(f))
            } else {
                Err(napi::Error::from_reason("number out of range for i64/f64"))
            }
        }
        serde_json::Value::String(s) => Ok(yydb::Value::Text(s)),
        other => Err(napi::Error::from_reason(format!(
            "unsupported scalar value: {other:?}"
        ))),
    }
}

fn value_to_json(value: yydb::Value) -> serde_json::Value {
    match value {
        yydb::Value::Null => serde_json::Value::Null,
        yydb::Value::Bool(b) => serde_json::Value::Bool(b),
        yydb::Value::I64(i) => serde_json::Value::Number(i.into()),
        yydb::Value::U64(u) => serde_json::Value::Number(u.into()),
        yydb::Value::F64(f) => serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        yydb::Value::Text(s) => serde_json::Value::String(s),
        other => serde_json::Value::String(format!("{other:?}")),
    }
}
