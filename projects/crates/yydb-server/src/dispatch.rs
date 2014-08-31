//! Request dispatch for `yydb serve`: wire frames mapped onto a `Connection`.
//!
//! Kept out of `yydb-protocol` because it executes database operations.

use yydb_protocol::{
    decode_kv_get, decode_kv_put, decode_micro_register, decode_scalar_call, decode_schema_ensure,
    encode_kv_get_ok, encode_scalar_call_ok, encode_schema_ok, error_frame, Frame, MessageType,
};
use yydb_store::Connection;
use yydb_types::{Error, Result};

fn info_body(conn: &Connection) -> Result<Vec<u8>> {
    let path = conn
        .path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<memory>".to_owned());
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

/// Dispatch one client request frame against an open [`Connection`].
pub fn dispatch(conn: &Connection, request: &Frame) -> Frame {
    let id = request.request_id;
    match request.msg_type {
        MessageType::Hello => Frame::new(
            MessageType::HelloOk,
            id,
            env!("CARGO_PKG_VERSION").as_bytes().to_vec(),
        ),
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
                Ok(value) => {
                    Frame::new(MessageType::ScalarCallOk, id, encode_scalar_call_ok(value))
                }
                Err(error) => error_frame(id, &error),
            },
            Err(error) => error_frame(id, &error),
        },
        MessageType::HelloOk
        | MessageType::InfoOk
        | MessageType::SchemaGetOk
        | MessageType::SchemaEnsureOk
        | MessageType::KvGetOk
        | MessageType::KvPutOk
        | MessageType::MicroRegisterOk
        | MessageType::MicroHostInvoke
        | MessageType::MicroHostInvokeOk
        | MessageType::ScalarCallOk
        | MessageType::Error => error_frame(
            id,
            &Error::Protocol {
                message: "unexpected client message type".to_owned(),
            },
        ),
        MessageType::Unknown(code) => error_frame(
            id,
            &Error::Protocol {
                message: format!("unknown msg_type {code}"),
            },
        ),
    }
}

/// Whether a `--bind` host is loopback (product-safe default).
pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim().trim_start_matches('[').trim_end_matches(']');
    matches!(
        host.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "::1" | "localhost" | "0:0:0:0:0:0:0:1"
    )
}
