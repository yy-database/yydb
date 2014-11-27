//! Request dispatch for `yydb serve`: wire frames mapped onto a `Connection`.
//!
//! Kept out of `yydb-protocol` because it executes database operations.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use yydb_engine::Connection;
use yydb_protocol::{
    Frame, MessageType,
    decode_authenticate, decode_execute_prepared, decode_kv_get, decode_kv_put, decode_micro_register,
    decode_prepare, decode_scalar_call, decode_schema_ensure,
    encode_authenticate_ok, encode_execute_prepared_ok, encode_kv_get_ok, encode_prepare_ok,
    encode_scalar_call_ok, encode_schema_ok, error_frame,
};
use yydb_query::PreparedStatement;
use yydb_types::{Error, Result};

// ─── Auth token ──────────────────────────────────────────────────────────

/// Hard-coded bearer token for the in-tree reference server.
///
/// **This is Phase-0 placeholder security.** Before any deployment, replace
/// this with a proper challenge-response handshake (e.g. shared-secret HMAC)
/// or mTLS. The contract is:
///
/// * token is verified on every `Authenticate` frame
/// * session tokens are opaque strings returned by the server
/// * both token values should be rotated per deployment (configurable via
///   env vars `YYDB_AUTH_TOKEN` / `YYDB_SESSION_TOKEN`)
fn expected_auth_token() -> &'static str {
    static TOKEN: OnceLock<String> = OnceLock::new();
    TOKEN.get_or_init(|| {
        std::env::var("YYDB_AUTH_TOKEN").unwrap_or_else(|_| "yydb-phase0-default-token".to_owned())
    })
}

fn session_token() -> &'static str {
    static TOKEN: OnceLock<String> = OnceLock::new();
    TOKEN.get_or_init(|| {
        std::env::var("YYDB_SESSION_TOKEN").unwrap_or_else(|_| "yydb-phase0-session-token".to_owned())
    })
}

// ─── Per-connection session state ─────────────────────────────────────────

/// A prepared statement stored by `dispatch` on the server side.
#[derive(Clone)]
pub struct PreparedPlan {
    /// The lowered plan with full op tree + parameter descriptors.
    pub statement: PreparedStatement,
    /// Connection that owns this plan (for authz auditing).
    pub connection_id: u64,
}

/// Per-connection state — tracks prepared statements and auth status.
pub struct SessionState {
    /// Monotonic id for this connection.
    pub connection_id: u64,
    /// Whether the connection has completed the Authenticate handshake.
    pub authenticated: Mutex<bool>,
    /// Plans prepared on this connection, keyed by `statement_id`.
    pub prepared: Mutex<BTreeMap<String, PreparedPlan>>,
}

impl SessionState {
    /// Fresh per-connection state; auth and plan registry start empty.
    pub fn new(connection_id: u64) -> Self {
        Self { connection_id, authenticated: Mutex::new(false), prepared: Mutex::new(BTreeMap::new()) }
    }
}

/// Read auth status for a connection (serve.rs needs this).
pub fn is_authenticated(state: &SessionState) -> bool {
    *state.authenticated.lock().expect("session auth poisoned")
}

/// Parse ExecutePrepared params JSON into a named-param map.
/// Returns Err(Error) for malformed JSON / wrong types.
fn parse_params_json(json: &str) -> Result<BTreeMap<String, yydb_types::Value>> {
    let parsed: serde_json::Value =
        serde_json::from_str(json).map_err(|e| Error::QueryBind { parameter: String::new(), message: format!("invalid params JSON: {e}") })?;
    let obj = parsed.as_object().ok_or_else(|| Error::QueryBind { parameter: String::new(), message: "params must be a JSON object".to_owned() })?;
    let mut m = BTreeMap::new();
    for (k, v) in obj {
        m.insert(k.clone(), json_to_value(v)?);
    }
    Ok(m)
}

// ─── Info body ───────────────────────────────────────────────────────────

fn info_body(conn: &Connection) -> Result<Vec<u8>> {
    let path = conn.path().map(|p| p.display().to_string()).unwrap_or_else(|| "<memory>".to_owned());
    let schema_line = match conn.schema()? {
        Some(schema) => format!("schema.version={}", schema.version),
        None => "schema.version=".to_owned(),
    };
    let text = format!(
        "path={path}\n{schema_line}\nrecords={}\njournal_mode={}\n",
        conn.record_count()?,
        conn.journal_mode().as_str()
    );
    Ok(text.into_bytes())
}

// ─── Dispatch ────────────────────────────────────────────────────────────

/// Dispatch one client request frame against an open [`Connection`], with
/// per-connection auth enforcement.
///
/// Connection lifecycle:
/// 1. `Hello` — always accepted (no auth required)
/// 2. `Authenticate` — verify token, mark `state.authenticated = true`
/// 3. Every other request — rejected with `Error { code: "yydb.auth.required" }`
///    until `authenticated == true`
///
/// Once authenticated, Prepare / ExecutePrepared hit the per-connection
/// plan registry; all other frames dispatch directly to the connection.
pub fn dispatch(conn: &Connection, state: &SessionState, request: &Frame) -> Frame {
    let id = request.request_id;
    match request.msg_type {
        // ── unauthenticated allowed ──────────────────────────────────────
        MessageType::Hello => Frame::new(MessageType::HelloOk, id, env!("CARGO_PKG_VERSION").as_bytes().to_vec()),
        MessageType::Authenticate => {
            let token = match decode_authenticate(&request.body) {
                Ok(t) => t,
                Err(error) => return error_frame(id, &error),
            };
            if token != expected_auth_token() {
                return error_frame(id, &Error::Auth {
                    message: "authentication failed: invalid token".to_owned(),
                });
            }
            *state.authenticated.lock().unwrap() = true;
            Frame::new(MessageType::AuthenticateOk, id, encode_authenticate_ok(session_token()))
        }
        // ── authenticated required ──────────────────────────────────────
        _ => {
            if !is_authenticated(state) {
                return error_frame(id, &Error::Auth {
                    message: format!("connection not authenticated — send Hello + Authenticate first (got {})", request.msg_type.as_u16()),
                });
            }
            dispatch_authed(conn, state, request)
        }
    }
}

fn dispatch_authed(conn: &Connection, state: &SessionState, request: &Frame) -> Frame {
    let id = request.request_id;
    match request.msg_type {
        MessageType::Info => match info_body(conn) {
            Ok(body) => Frame::new(MessageType::InfoOk, id, body),
            Err(error) => error_frame(id, &error),
        },
        MessageType::SchemaGet => match conn.schema() {
            Ok(schema) => Frame::new(MessageType::SchemaGetOk, id, encode_schema_ok(schema)),
            Err(error) => error_frame(id, &error),
        },
        MessageType::SchemaEnsure => match decode_schema_ensure(&request.body) {
            Ok((_version, document)) => match conn.ensure_schema(&document) {
                Ok(()) => Frame::new(MessageType::SchemaEnsureOk, id, Vec::new()),
                Err(error) => error_frame(id, &error),
            },
            Err(error) => error_frame(id, &error),
        },
        MessageType::KvGet => match decode_kv_get(&request.body) {
            Ok(key) => match conn.get(&key) {
                Ok(value) => Frame::new(MessageType::KvGetOk, id, encode_kv_get_ok(value)),
                Err(error) => error_frame(id, &error),
            },
            Err(error) => error_frame(id, &error),
        },
        MessageType::KvPut => match decode_kv_put(&request.body) {
            Ok((key, value)) => match conn.put(key, value) {
                Ok(()) => Frame::new(MessageType::KvPutOk, id, Vec::new()),
                Err(error) => error_frame(id, &error),
            },
            Err(error) => error_frame(id, &error),
        },
        MessageType::MicroRegister => match decode_micro_register(&request.body) {
            Ok(definition) => match conn.register_host_micro(definition) {
                Ok(()) => Frame::new(MessageType::MicroRegisterOk, id, Vec::new()),
                Err(error) => error_frame(id, &error),
            },
            Err(error) => error_frame(id, &error),
        },
        MessageType::ScalarCall => match decode_scalar_call(&request.body) {
            Ok((name, version, args)) => match conn.call_scalar_version(&name, version, &args) {
                Ok(value) => Frame::new(MessageType::ScalarCallOk, id, encode_scalar_call_ok(value)),
                Err(error) => error_frame(id, &error),
            },
            Err(error) => error_frame(id, &error),
        },
        // ── Prepared statement path ──────────────────────────────────────
        MessageType::Prepare => {
            let source = match decode_prepare(&request.body) {
                Ok(s) => s,
                Err(error) => return error_frame(id, &error),
            };
            let stmt = match conn.prepare_vos(&source) {
                Ok(s) => s,
                Err(error) => return error_frame(id, &error),
            };
            let sid = stmt.statement_id.clone();
            state.prepared.lock().unwrap().insert(sid.clone(), PreparedPlan {
                statement: stmt,
                connection_id: state.connection_id,
            });
            Frame::new(MessageType::PrepareOk, id, encode_prepare_ok(&sid))
        }
        MessageType::ExecutePrepared => {
            let (sid, params_json) = match decode_execute_prepared(&request.body) {
                Ok(v) => v,
                Err(error) => return error_frame(id, &error),
            };
            let plan = match state.prepared.lock().unwrap().get(&sid).cloned() {
                Some(p) => p,
                None => return error_frame(id, &Error::QueryBind {
                    parameter: String::new(),
                    message: format!("unknown prepared statement `{sid}` — must Prepare first on this connection"),
                }),
            };
            let named_params = match params_json {
                Some(json) => match parse_params_json(&json) {
                    Ok(m) => m,
                    Err(error) => return error_frame(id, &error),
                },
                None => BTreeMap::new(),
            };
            let rows = match conn.query_prepared(&plan.statement, &named_params) {
                Ok(r) => r,
                Err(error) => return error_frame(id, &error),
            };
            let json_rows = rows_to_json(&rows);
            Frame::new(MessageType::ExecutePreparedOk, id, encode_execute_prepared_ok(&json_rows))
        }
        // ── server→client messages / handshake-only frames we never dispatch here ──
        MessageType::HelloOk
        | MessageType::AuthenticateOk
        | MessageType::InfoOk
        | MessageType::SchemaGetOk
        | MessageType::SchemaEnsureOk
        | MessageType::KvGetOk
        | MessageType::KvPutOk
        | MessageType::MicroRegisterOk
        | MessageType::MicroHostInvoke
        | MessageType::MicroHostInvokeOk
        | MessageType::ScalarCallOk
        | MessageType::PrepareOk
        | MessageType::ExecutePreparedOk
        | MessageType::Error => error_frame(id, &Error::Protocol { message: "unexpected client message type".to_owned() }),
        // Hello / Authenticate are handled by dispatch() before auth check — reaching here means
        // they somehow slipped through; reject defensively.
        MessageType::Hello | MessageType::Authenticate => error_frame(id, &Error::Protocol {
            message: "handshake frame received after auth — protocol violation".to_owned(),
        }),
        MessageType::Unknown(code) => error_frame(id, &Error::Protocol { message: format!("unknown msg_type {code}") }),
    }
}

/// Whether a `--bind` host is loopback (product-safe default).
pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim().trim_start_matches('[').trim_end_matches(']');
    matches!(host.to_ascii_lowercase().as_str(), "127.0.0.1" | "::1" | "localhost" | "0:0:0:0:0:0:0:1")
}

/// Convert a `serde_json::Value` (from the on-wire params object) to a
/// `yydb_types::Value`. Only the wire-safe scalar subset is supported.
fn json_to_value(v: &serde_json::Value) -> Result<yydb_types::Value> {
    use yydb_types::Value;
    match v {
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::Bool(b) => Ok(Value::Bool(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() { Ok(Value::I64(i)) }
            else if let Some(f) = n.as_f64() { Ok(Value::F64(f)) }
            else { Err(Error::QueryBind {
                parameter: String::new(),
                message: format!("unsupported JSON number: {n}"),
            })}
        }
        serde_json::Value::String(s) => Ok(Value::Text(s.clone())),
        _ => Err(Error::QueryBind {
            parameter: String::new(),
            message: format!("unsupported JSON value type for parameter: {v:?}"),
        }),
    }
}

/// Serialize query rows to compact JSON. Each row becomes `{ field: value, … }`.
fn rows_to_json(rows: &[std::collections::BTreeMap<String, yydb_types::Value>]) -> String {
    use yydb_types::Value;
    let mut out = String::from("[");
    for (i, row) in rows.iter().enumerate() {
        if i > 0 { out.push(','); }
        out.push('{');
        for (j, (k, v)) in row.iter().enumerate() {
            if j > 0 { out.push(','); }
            out.push_str(&format!("\"{}\":", escape_json(k)));
            match v {
                Value::Null => out.push_str("null"),
                Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
                Value::I64(n) => out.push_str(&n.to_string()),
                Value::U64(n) => out.push_str(&n.to_string()),
                Value::F64(n) => out.push_str(&n.to_string()),
                Value::Text(s) => out.push_str(&format!("\"{}\"", escape_json(s))),
                _ => out.push_str("null"), // complex types not serialized on wire
            }
        }
        out.push('}');
    }
    out.push(']');
    out
}

fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}
