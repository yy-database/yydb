//! YY wire protocol: frame layout, message bodies and the `0000` version prefix.
//!
//! Shared by `yydb-server` and `yydb-client`. This crate never opens a database,
//! never dispatches a request and never depends on the embedded engine.

#![deny(missing_docs)]

use std::io::{Read, Write};

use yydb_types::udf::{HostFunctionHandle, UdfValue};
use yydb_types::{Error, Result, SchemaVersion, Value};

/// Product magic for YYDB hosts (self-claim; same wire as `YYDS`).
pub const MAGIC_YYDB: &[u8; 4] = b"YYDB";

/// Product magic for YYDS hosts (self-claim; same wire as `YYDB`).
pub const MAGIC_YYDS: &[u8; 4] = b"YYDS";

/// Current wire version digits (`0000`).
pub const WIRE_VERSION: &[u8; 4] = b"0000";

/// Next planned wire version digits (not accepted yet).
pub const WIRE_VERSION_NEXT: &[u8; 4] = b"0001";

/// Absolute header size in bytes (`magic` + `version` + fields).
pub const HEADER_LEN: usize = 20;

/// Soft cap on body size for the reference server / client.
pub const MAX_BODY_LEN: u32 = 16 * 1024 * 1024;

/// Wire message type codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    /// Client hello.
    Hello,
    /// Server hello reply (version string).
    HelloOk,
    /// Database info request.
    Info,
    /// Database info reply.
    InfoOk,
    /// Fetch schema.
    SchemaGet,
    /// Schema payload.
    SchemaGetOk,
    /// Ensure schema.
    SchemaEnsure,
    /// Ensure schema ack.
    SchemaEnsureOk,
    /// KV get.
    KvGet,
    /// KV get reply.
    KvGetOk,
    /// KV put.
    KvPut,
    /// KV put ack.
    KvPutOk,
    /// Register a session-local host micro (metadata only).
    MicroRegister,
    /// Host micro registration ack.
    MicroRegisterOk,
    /// Server asks the wire peer to run a host micro (S→C).
    MicroHostInvoke,
    /// Host micro result (C→S).
    MicroHostInvokeOk,
    /// Invoke a registered scalar UDF (C→S).
    ScalarCall,
    /// Scalar UDF result (S→C).
    ScalarCallOk,
    /// Error reply.
    Error,
    /// Unrecognized type (still framed so the peer can answer with Error).
    Unknown(u16),
}

impl MessageType {
    /// Parse a raw `u16` code.
    pub fn from_u16(value: u16) -> Self {
        match value {
            1 => Self::Hello,
            2 => Self::HelloOk,
            3 => Self::Info,
            4 => Self::InfoOk,
            5 => Self::SchemaGet,
            6 => Self::SchemaGetOk,
            7 => Self::SchemaEnsure,
            8 => Self::SchemaEnsureOk,
            9 => Self::KvGet,
            10 => Self::KvGetOk,
            11 => Self::KvPut,
            12 => Self::KvPutOk,
            13 => Self::MicroRegister,
            14 => Self::MicroRegisterOk,
            15 => Self::MicroHostInvoke,
            16 => Self::MicroHostInvokeOk,
            17 => Self::ScalarCall,
            18 => Self::ScalarCallOk,
            255 => Self::Error,
            other => Self::Unknown(other),
        }
    }

    /// Raw code.
    pub fn as_u16(self) -> u16 {
        match self {
            Self::Hello => 1,
            Self::HelloOk => 2,
            Self::Info => 3,
            Self::InfoOk => 4,
            Self::SchemaGet => 5,
            Self::SchemaGetOk => 6,
            Self::SchemaEnsure => 7,
            Self::SchemaEnsureOk => 8,
            Self::KvGet => 9,
            Self::KvGetOk => 10,
            Self::KvPut => 11,
            Self::KvPutOk => 12,
            Self::MicroRegister => 13,
            Self::MicroRegisterOk => 14,
            Self::MicroHostInvoke => 15,
            Self::MicroHostInvokeOk => 16,
            Self::ScalarCall => 17,
            Self::ScalarCallOk => 18,
            Self::Error => 255,
            Self::Unknown(code) => code,
        }
    }
}

/// One length-prefixed binary frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Message type.
    pub msg_type: MessageType,
    /// Reserved flags (must be 0 in v0).
    pub flags: u16,
    /// Client-chosen request id (echoed on replies).
    pub request_id: u32,
    /// Payload bytes.
    pub body: Vec<u8>,
}

impl Frame {
    /// Build a frame with empty flags.
    pub fn new(msg_type: MessageType, request_id: u32, body: impl Into<Vec<u8>>) -> Self {
        Self {
            msg_type,
            flags: 0,
            request_id,
            body: body.into(),
        }
    }

    /// Encode into the on-wire byte layout (product magic `YYDB`).
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.encode_as(MAGIC_YYDB)
    }

    /// Encode with an explicit product magic (`YYDB` or `YYDS`).
    pub fn encode_as(&self, product: &[u8; 4]) -> Result<Vec<u8>> {
        if product != MAGIC_YYDB && product != MAGIC_YYDS {
            return Err(Error::Unsupported(
                "wire product magic must be YYDB or YYDS",
            ));
        }
        if self.body.len() > MAX_BODY_LEN as usize {
            return Err(Error::Unsupported("wire body too large"));
        }
        let mut out = Vec::with_capacity(HEADER_LEN + self.body.len());
        out.extend_from_slice(product);
        out.extend_from_slice(WIRE_VERSION);
        out.extend_from_slice(&self.msg_type.as_u16().to_le_bytes());
        out.extend_from_slice(&self.flags.to_le_bytes());
        out.extend_from_slice(&self.request_id.to_le_bytes());
        out.extend_from_slice(&(self.body.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.body);
        Ok(out)
    }

    /// Decode one frame from a complete buffer (header + body).
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::Corrupt("wire frame truncated header"));
        }
        check_prefix(&bytes[..8])?;
        let msg_raw = u16::from_le_bytes([bytes[8], bytes[9]]);
        let msg_type = MessageType::from_u16(msg_raw);
        let flags = u16::from_le_bytes([bytes[10], bytes[11]]);
        let request_id = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        let body_len = u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        if body_len > MAX_BODY_LEN {
            return Err(Error::Unsupported("wire body too large"));
        }
        let need = HEADER_LEN + body_len as usize;
        if bytes.len() < need {
            return Err(Error::Corrupt("wire frame truncated body"));
        }
        Ok(Self {
            msg_type,
            flags,
            request_id,
            body: bytes[HEADER_LEN..need].to_vec(),
        })
    }
}

fn check_prefix(prefix: &[u8]) -> Result<()> {
    if prefix.len() < 8 {
        return Err(Error::Corrupt("wire frame truncated magic/version"));
    }
    let magic = &prefix[0..4];
    let version = &prefix[4..8];
    if magic != MAGIC_YYDB && magic != MAGIC_YYDS {
        return Err(Error::Corrupt(
            "wire frame bad product magic (want YYDB|YYDS)",
        ));
    }
    if version != WIRE_VERSION {
        return Err(Error::Unsupported("wire version not supported (want 0000)"));
    }
    Ok(())
}

/// Read exactly one frame from a blocking reader.
pub fn read_frame<R: Read>(reader: &mut R) -> Result<Frame> {
    let mut header = [0_u8; HEADER_LEN];
    reader.read_exact(&mut header)?;
    check_prefix(&header)?;
    let body_len = u32::from_le_bytes([header[16], header[17], header[18], header[19]]);
    if body_len > MAX_BODY_LEN {
        return Err(Error::Unsupported("wire body too large"));
    }
    let mut body = vec![0_u8; body_len as usize];
    if body_len > 0 {
        reader.read_exact(&mut body)?;
    }
    let mut full = Vec::with_capacity(HEADER_LEN + body.len());
    full.extend_from_slice(&header);
    full.extend_from_slice(&body);
    Frame::decode(&full)
}

/// Write one frame to a blocking writer.
pub fn write_frame<W: Write>(writer: &mut W, frame: &Frame) -> Result<()> {
    let bytes = frame.encode()?;
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

fn read_u32(buf: &[u8], offset: &mut usize) -> Result<u32> {
    if *offset + 4 > buf.len() {
        return Err(Error::Corrupt("wire body truncated u32"));
    }
    let value = u32::from_le_bytes([
        buf[*offset],
        buf[*offset + 1],
        buf[*offset + 2],
        buf[*offset + 3],
    ]);
    *offset += 4;
    Ok(value)
}

fn read_bytes<'a>(buf: &'a [u8], offset: &mut usize, len: u32) -> Result<&'a [u8]> {
    let len = len as usize;
    if *offset + len > buf.len() {
        return Err(Error::Corrupt("wire body truncated bytes"));
    }
    let slice = &buf[*offset..*offset + len];
    *offset += len;
    Ok(slice)
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    push_u32(out, bytes.len() as u32);
    out.extend_from_slice(bytes);
}

/// Build an `Error` reply frame.
pub fn error_frame(request_id: u32, error: &Error) -> Frame {
    Frame::new(MessageType::Error, request_id, encode_error_body(error))
}

/// Encode a `SchemaGetOk` body.
pub fn encode_schema_ok(schema: Option<SchemaVersion>) -> Vec<u8> {
    match schema {
        None => vec![0],
        Some(schema) => {
            let mut body = vec![1];
            push_u32(&mut body, schema.version);
            push_bytes(&mut body, schema.document.as_bytes());
            body
        }
    }
}

/// Decode a `SchemaEnsure` body.
pub fn decode_schema_ensure(body: &[u8]) -> Result<(u32, String)> {
    let mut offset = 0;
    let version = read_u32(body, &mut offset)?;
    let doc_len = read_u32(body, &mut offset)?;
    let doc = read_bytes(body, &mut offset, doc_len)?;
    let document = std::str::from_utf8(doc)
        .map_err(|_| Error::Corrupt("schema document is not utf-8"))?
        .to_owned();
    Ok((version, document))
}

/// Decode a `KvGet` body.
pub fn decode_kv_get(body: &[u8]) -> Result<String> {
    let mut offset = 0;
    let key_len = read_u32(body, &mut offset)?;
    let key = read_bytes(body, &mut offset, key_len)?;
    Ok(std::str::from_utf8(key)
        .map_err(|_| Error::Corrupt("kv key is not utf-8"))?
        .to_owned())
}

/// Decode a `KvPut` body.
pub fn decode_kv_put(body: &[u8]) -> Result<(String, Vec<u8>)> {
    let mut offset = 0;
    let key_len = read_u32(body, &mut offset)?;
    let key = read_bytes(body, &mut offset, key_len)?;
    let key = std::str::from_utf8(key)
        .map_err(|_| Error::Corrupt("kv key is not utf-8"))?
        .to_owned();
    let val_len = read_u32(body, &mut offset)?;
    let value = read_bytes(body, &mut offset, val_len)?.to_vec();
    Ok((key, value))
}

/// Encode a `KvGetOk` body.
pub fn encode_kv_get_ok(value: Option<Vec<u8>>) -> Vec<u8> {
    match value {
        None => vec![0],
        Some(bytes) => {
            let mut body = vec![1];
            push_bytes(&mut body, &bytes);
            body
        }
    }
}

/// Encode a `SchemaEnsure` body.
pub fn encode_schema_ensure(version: u32, document: &str) -> Vec<u8> {
    let mut body = Vec::new();
    push_u32(&mut body, version);
    push_bytes(&mut body, document.as_bytes());
    body
}

/// Encode a `KvGet` body.
pub fn encode_kv_get(key: &str) -> Vec<u8> {
    let mut body = Vec::new();
    push_bytes(&mut body, key.as_bytes());
    body
}

fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn read_u64(buf: &[u8], offset: &mut usize) -> Result<u64> {
    if *offset + 8 > buf.len() {
        return Err(Error::Corrupt("wire body truncated u64"));
    }
    let value = u64::from_le_bytes([
        buf[*offset],
        buf[*offset + 1],
        buf[*offset + 2],
        buf[*offset + 3],
        buf[*offset + 4],
        buf[*offset + 5],
        buf[*offset + 6],
        buf[*offset + 7],
    ]);
    *offset += 8;
    Ok(value)
}

fn encode_udf_type_tag(ty: yydb_types::udf::UdfType) -> u8 {
    match ty {
        yydb_types::udf::UdfType::Null => 0,
        yydb_types::udf::UdfType::Bool => 1,
        yydb_types::udf::UdfType::I64 => 2,
        yydb_types::udf::UdfType::Text => 3,
        yydb_types::udf::UdfType::Any => {
            unreachable!("UdfType::Any is not encodable on the wire")
        }
    }
}

fn decode_udf_type_tag(tag: u8) -> Result<yydb_types::udf::UdfType> {
    match tag {
        0 => Ok(yydb_types::udf::UdfType::Null),
        1 => Ok(yydb_types::udf::UdfType::Bool),
        2 => Ok(yydb_types::udf::UdfType::I64),
        3 => Ok(yydb_types::udf::UdfType::Text),
        _ => Err(Error::Corrupt("wire scalar value has invalid type tag")),
    }
}

fn encode_udf_value(out: &mut Vec<u8>, value: &UdfValue) {
    match value {
        UdfValue::Null => out.push(0),
        UdfValue::Bool(value) => {
            out.push(1);
            out.push(if *value { 1 } else { 0 });
        }
        UdfValue::I64(value) => {
            out.push(2);
            out.extend_from_slice(&value.to_le_bytes());
        }
        UdfValue::Text(value) => {
            out.push(3);
            push_bytes(out, value.as_bytes());
        }
    }
}

fn decode_udf_value(body: &[u8], offset: &mut usize) -> Result<UdfValue> {
    if *offset >= body.len() {
        return Err(Error::Corrupt("wire scalar value missing tag"));
    }
    let tag = body[*offset];
    *offset += 1;
    match tag {
        0 => Ok(UdfValue::Null),
        1 => {
            if *offset >= body.len() {
                return Err(Error::Corrupt("wire bool value truncated"));
            }
            let value = body[*offset] != 0;
            *offset += 1;
            Ok(UdfValue::Bool(value))
        }
        2 => {
            if *offset + 8 > body.len() {
                return Err(Error::Corrupt("wire i64 value truncated"));
            }
            let value = i64::from_le_bytes([
                body[*offset],
                body[*offset + 1],
                body[*offset + 2],
                body[*offset + 3],
                body[*offset + 4],
                body[*offset + 5],
                body[*offset + 6],
                body[*offset + 7],
            ]);
            *offset += 8;
            Ok(UdfValue::I64(value))
        }
        3 => {
            let len = read_u32(body, offset)?;
            let bytes = read_bytes(body, offset, len)?;
            let text = std::str::from_utf8(bytes)
                .map_err(|_| Error::Corrupt("wire text value is not utf-8"))?
                .to_owned();
            Ok(UdfValue::Text(text))
        }
        _ => Err(Error::Corrupt("wire scalar value has invalid type tag")),
    }
}

fn encode_udf_values(values: &[UdfValue]) -> Vec<u8> {
    let mut body = Vec::new();
    assert!(values.len() <= 255, "wire scalar arg count exceeds u8");
    body.push(values.len() as u8);
    for value in values {
        encode_udf_value(&mut body, value);
    }
    body
}

fn decode_udf_values(body: &[u8], offset: &mut usize) -> Result<Vec<UdfValue>> {
    if *offset >= body.len() {
        return Err(Error::Corrupt("wire scalar args missing count"));
    }
    let count = body[*offset] as usize;
    *offset += 1;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(decode_udf_value(body, offset)?);
    }
    Ok(values)
}

fn udf_value_to_runtime(value: UdfValue) -> Result<Value> {
    match value {
        UdfValue::Null => Ok(Value::Null),
        UdfValue::Bool(value) => Ok(Value::Bool(value)),
        UdfValue::I64(value) => Ok(Value::I64(value)),
        UdfValue::Text(value) => Ok(Value::Text(value)),
    }
}

fn runtime_value_to_udf(value: &Value) -> Result<UdfValue> {
    match value {
        Value::Null => Ok(UdfValue::Null),
        Value::Bool(value) => Ok(UdfValue::Bool(*value)),
        Value::I64(value) => Ok(UdfValue::I64(*value)),
        Value::Text(value) => Ok(UdfValue::Text(value.clone())),
        _ => Err(Error::Unsupported(
            "scalar call value type is outside the Phase-1 wire subset",
        )),
    }
}

/// Encode a `MicroHostInvoke` body.
pub fn encode_micro_host_invoke(handle: &HostFunctionHandle, args: &[UdfValue]) -> Vec<u8> {
    let mut body = Vec::new();
    push_u64(&mut body, handle.host_id);
    push_u32(&mut body, handle.version);
    push_bytes(&mut body, handle.function_id.as_bytes());
    body.extend_from_slice(&encode_udf_values(args));
    body
}

/// Decode a `MicroHostInvoke` body.
pub fn decode_micro_host_invoke(body: &[u8]) -> Result<(HostFunctionHandle, Vec<UdfValue>)> {
    let mut offset = 0;
    let host_id = read_u64(body, &mut offset)?;
    let handle_version = read_u32(body, &mut offset)?;
    let function_id_len = read_u32(body, &mut offset)?;
    let function_id = read_bytes(body, &mut offset, function_id_len)?;
    let function_id = std::str::from_utf8(function_id)
        .map_err(|_| Error::Corrupt("micro host invoke function_id is not utf-8"))?
        .to_owned();
    let args = decode_udf_values(body, &mut offset)?;
    Ok((
        HostFunctionHandle {
            host_id,
            function_id,
            version: handle_version,
        },
        args,
    ))
}

/// Encode a `MicroHostInvokeOk` body.
pub fn encode_micro_host_invoke_ok(value: UdfValue) -> Vec<u8> {
    let mut body = Vec::new();
    encode_udf_value(&mut body, &value);
    body
}

/// Decode a `MicroHostInvokeOk` body.
pub fn decode_micro_host_invoke_ok(body: &[u8]) -> Result<UdfValue> {
    let mut offset = 0;
    decode_udf_value(body, &mut offset)
}

/// Encode a `ScalarCall` body.
pub fn encode_scalar_call(name: &str, version: u32, args: &[Value]) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    push_u32(&mut body, version);
    push_bytes(&mut body, name.as_bytes());
    let mut udf_args = Vec::with_capacity(args.len());
    for value in args {
        udf_args.push(runtime_value_to_udf(value)?);
    }
    body.extend_from_slice(&encode_udf_values(&udf_args));
    Ok(body)
}

/// Decode a `ScalarCall` body.
pub fn decode_scalar_call(body: &[u8]) -> Result<(String, u32, Vec<Value>)> {
    let mut offset = 0;
    let version = read_u32(body, &mut offset)?;
    let name_len = read_u32(body, &mut offset)?;
    let name = read_bytes(body, &mut offset, name_len)?;
    let name = std::str::from_utf8(name)
        .map_err(|_| Error::Corrupt("scalar call name is not utf-8"))?
        .to_owned();
    let udf_args = decode_udf_values(body, &mut offset)?;
    let mut args = Vec::with_capacity(udf_args.len());
    for value in udf_args {
        args.push(udf_value_to_runtime(value)?);
    }
    Ok((name, version, args))
}

/// Encode a `ScalarCallOk` body.
pub fn encode_scalar_call_ok(value: Value) -> Vec<u8> {
    let udf_value = runtime_value_to_udf(&value).expect("scalar call ok value must be wire-safe");
    encode_micro_host_invoke_ok(udf_value)
}

/// Decode a `ScalarCallOk` body.
pub fn decode_scalar_call_ok(body: &[u8]) -> Result<Value> {
    udf_value_to_runtime(decode_micro_host_invoke_ok(body)?)
}

/// Encode a `MicroRegister` body.
pub fn encode_micro_register(definition: &yydb_types::udf::HostMicroDefinition) -> Vec<u8> {
    let mut body = Vec::new();
    push_u64(&mut body, definition.handle.host_id);
    push_u32(&mut body, definition.handle.version);
    push_u32(&mut body, definition.identity.version);
    push_bytes(&mut body, definition.identity.name().as_bytes());
    push_bytes(&mut body, definition.handle.function_id.as_bytes());
    let arg_count = definition.signature.args.len();
    body.push(arg_count as u8);
    for ty in &definition.signature.args {
        body.push(encode_udf_type_tag(*ty));
    }
    body.push(encode_udf_type_tag(definition.signature.returns));
    body.extend_from_slice(&definition.fingerprint);
    body
}

/// Decode a `MicroRegister` body.
pub fn decode_micro_register(body: &[u8]) -> Result<yydb_types::udf::HostMicroDefinition> {
    let mut offset = 0;
    let host_id = read_u64(body, &mut offset)?;
    let handle_version = read_u32(body, &mut offset)?;
    let udf_version = read_u32(body, &mut offset)?;
    let name_len = read_u32(body, &mut offset)?;
    let name = read_bytes(body, &mut offset, name_len)?;
    let name = std::str::from_utf8(name)
        .map_err(|_| Error::Corrupt("micro register name is not utf-8"))?;
    let function_id_len = read_u32(body, &mut offset)?;
    let function_id = read_bytes(body, &mut offset, function_id_len)?;
    let function_id = std::str::from_utf8(function_id)
        .map_err(|_| Error::Corrupt("micro register function_id is not utf-8"))?
        .to_owned();
    if offset >= body.len() {
        return Err(Error::Corrupt("micro register missing arg count"));
    }
    let arg_count = body[offset] as usize;
    offset += 1;
    let mut args = Vec::with_capacity(arg_count);
    for _ in 0..arg_count {
        if offset >= body.len() {
            return Err(Error::Corrupt("micro register truncated arg types"));
        }
        args.push(decode_udf_type_tag(body[offset])?);
        offset += 1;
    }
    if offset >= body.len() {
        return Err(Error::Corrupt("micro register missing return type"));
    }
    let returns = decode_udf_type_tag(body[offset])?;
    offset += 1;
    if offset + 32 > body.len() {
        return Err(Error::Corrupt("micro register missing fingerprint"));
    }
    let mut fingerprint = [0_u8; 32];
    fingerprint.copy_from_slice(&body[offset..offset + 32]);
    yydb_types::udf::HostMicroDefinition::from_scalar(
        name,
        udf_version,
        args,
        returns,
        host_id,
        function_id,
        handle_version,
        fingerprint,
    )
    .map_err(|error| Error::Udf {
        name: name.to_owned(),
        message: error.to_string(),
    })
}

/// Encode a `KvPut` body.
pub fn encode_kv_put(key: &str, value: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    push_bytes(&mut body, key.as_bytes());
    push_bytes(&mut body, value);
    body
}

/// Decode `SchemaGetOk` body.
pub fn decode_schema_get_ok(body: &[u8]) -> Result<Option<SchemaVersion>> {
    if body.is_empty() {
        return Err(Error::Corrupt("schema get ok empty"));
    }
    if body[0] == 0 {
        return Ok(None);
    }
    let mut offset = 1;
    let version = read_u32(body, &mut offset)?;
    let doc_len = read_u32(body, &mut offset)?;
    let doc = read_bytes(body, &mut offset, doc_len)?;
    let document = std::str::from_utf8(doc)
        .map_err(|_| Error::Corrupt("schema document is not utf-8"))?
        .to_owned();
    Ok(Some(SchemaVersion { version, document }))
}

/// Decode `KvGetOk` body.
pub fn decode_kv_get_ok(body: &[u8]) -> Result<Option<Vec<u8>>> {
    if body.is_empty() {
        return Err(Error::Corrupt("kv get ok empty"));
    }
    if body[0] == 0 {
        return Ok(None);
    }
    let mut offset = 1;
    let val_len = read_u32(body, &mut offset)?;
    Ok(Some(read_bytes(body, &mut offset, val_len)?.to_vec()))
}

/// One structured `Error` frame body: code plus human-readable message.
///
/// See [`Error::to_json`] for the full diagnostic contract (code, operation,
/// retryable, params, message). [`encode_error_body`] and [`decode_error`]
/// serialize / deserialize that compact JSON object on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireError {
    /// Stable error code (`yydb.schema.invalid`, …); see [`Error::code`].
    pub code: String,
    /// Human-readable message reported by the peer. Hosts must not branch on
    /// this text; branch on [`Self::code`].
    pub message: String,
    /// High-level operation category (`"schema"`, `"udf"`, …).
    pub operation: String,
    /// Whether the caller may retry after backing off.
    pub retryable: bool,
    /// Optional structured key/value parameters for catalog templating.
    pub params: Option<std::collections::BTreeMap<String, String>>,
}

/// Encode an [`Error`] into an `Error` frame body as compact diagnostic JSON.
pub fn encode_error_body(error: &Error) -> Vec<u8> {
    let json = serde_json::to_vec(&error.to_json())
        .expect("YYDB diagnostic JSON must never fail to encode");
    json
}

/// Decode an `Error` frame body from diagnostic JSON.
pub fn decode_error(body: &[u8]) -> Result<WireError> {
    let value: serde_json::Value = serde_json::from_slice(body)
        .map_err(|_| Error::Corrupt("wire error body is not valid JSON"))?;
    let code = value["code"]
        .as_str()
        .ok_or_else(|| Error::Corrupt("wire error body missing code"))?
        .to_owned();
    let message = value["message"]
        .as_str()
        .ok_or_else(|| Error::Corrupt("wire error body missing message"))?
        .to_owned();
    let operation = value["operation"]
        .as_str()
        .ok_or_else(|| Error::Corrupt("wire error body missing operation"))?
        .to_owned();
    let retryable = value["retryable"]
        .as_bool()
        .ok_or_else(|| Error::Corrupt("wire error body missing retryable"))?;
    let params = value["params"].as_object().map(|obj| {
        obj.iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
            .collect()
    });
    Ok(WireError {
        code,
        message,
        operation,
        retryable,
        params,
    })
}

/// Split `host:port` / `[::1]:port` into host and port string.
pub fn split_bind(bind: &str) -> Result<(String, u16)> {
    let bind = bind.trim();
    if let Some(rest) = bind.strip_prefix('[') {
        let (host, port) = rest
            .split_once("]:")
            .ok_or(Error::Unsupported("bind must look like [host]:port"))?;
        let port: u16 = port
            .parse()
            .map_err(|_| Error::Unsupported("bind port is not a u16"))?;
        return Ok((host.to_owned(), port));
    }
    let (host, port) = bind
        .rsplit_once(':')
        .ok_or(Error::Unsupported("bind must look like host:port"))?;
    let port: u16 = port
        .parse()
        .map_err(|_| Error::Unsupported("bind port is not a u16"))?;
    Ok((host.to_owned(), port))
}
