//! YY wire protocol surface for wasm hosts.
//!
//! Every frame layout and message-body codec is the upstream
//! `database.protocol.yy` implementation; this module only bridges it onto
//! wasm-bindgen types. No wire constant, header layout or body encoding is
//! re-declared here.

use database::{protocol::yy as wire, types::DiagnosticEnvelope, udf::{UdfType, UdfValue}};
use wasm_bindgen::prelude::*;

use crate::session;

fn js_error(error: database::types::Error) -> JsValue {
    JsValue::from_str(&error.to_string())
}

fn product_magic(product: &str) -> std::result::Result<&'static [u8; 4], JsValue> {
    match product {
        "YYDB" => Ok(wire::MAGIC_YYDB),
        "YYDS" => Ok(wire::MAGIC_YYDS),
        other => Err(JsValue::from_str(&format!("wire product magic must be YYDB or YYDS (got {other})"))),
    }
}

fn udf_type_tag(ty: UdfType) -> &'static str {
    match ty {
        UdfType::Null => "null",
        UdfType::Bool => "bool",
        UdfType::I64 => "i64",
        UdfType::Text => "text",
        UdfType::Any => "any",
    }
}

fn udf_value_json(value: &UdfValue) -> serde_json::Value {
    match value {
        UdfValue::Null => serde_json::Value::Null,
        UdfValue::Bool(value) => serde_json::Value::Bool(*value),
        UdfValue::I64(value) => serde_json::Value::from(*value),
        UdfValue::Text(value) => serde_json::Value::String(value.clone()),
    }
}

fn udf_value_from_json(value: &serde_json::Value) -> std::result::Result<UdfValue, JsValue> {
    match value {
        serde_json::Value::Null => Ok(UdfValue::Null),
        serde_json::Value::Bool(value) => Ok(UdfValue::Bool(*value)),
        serde_json::Value::Number(number) => number
            .as_i64()
            .map(UdfValue::I64)
            .ok_or_else(|| JsValue::from_str("wire udf value must be an integer i64 number")),
        serde_json::Value::String(value) => Ok(UdfValue::Text(value.clone())),
        _ => Err(JsValue::from_str("wire udf value must be null, bool, integer or string")),
    }
}

/// Current wire version digits (`"0000"`).
#[wasm_bindgen(js_name = wireVersion)]
pub fn wire_version() -> String {
    std::str::from_utf8(wire::WIRE_VERSION).expect("wire version digits are ASCII").to_owned()
}

/// Next planned wire version digits (`"0001"`, not spoken yet).
#[wasm_bindgen(js_name = wireVersionNext)]
pub fn wire_version_next() -> String {
    std::str::from_utf8(wire::WIRE_VERSION_NEXT).expect("wire version digits are ASCII").to_owned()
}

/// Four-byte product magic for `"YYDB"` or `"YYDS"`.
#[wasm_bindgen(js_name = wireMagic)]
pub fn wire_magic(product: &str) -> std::result::Result<Vec<u8>, JsValue> {
    product_magic(product).map(|magic| magic.to_vec())
}

/// Absolute frame header length in bytes.
#[wasm_bindgen(js_name = wireHeaderLen)]
pub fn wire_header_len() -> usize {
    wire::HEADER_LEN
}

/// Body soft limit in bytes enforced by frame encode and decode.
#[wasm_bindgen(js_name = wireMaxBodyLen)]
pub fn wire_max_body_len() -> u32 {
    wire::MAX_BODY_LEN
}

/// Response type a request code must be answered with, or `null` when the
/// code is not a request (responses, `Error` and unknown codes yield `null`).
#[wasm_bindgen(js_name = responseType)]
pub fn response_type(code: u16) -> Option<u16> {
    wire::response_type(wire::MessageType::from_u16(code)).map(wire::MessageType::as_u16)
}

/// Encode one wire frame over the upstream layout.
///
/// `product` is the sender self-claim (`"YYDB"` or `"YYDS"`; same protocol).
/// Codes outside the upstream message table are rejected: hosts must send
/// unknown types only as received, never invent them.
#[wasm_bindgen(js_name = frameEncode)]
pub fn frame_encode(
    msg_type: u16,
    flags: u16,
    request_id: u32,
    body: &[u8],
    product: &str,
) -> std::result::Result<Vec<u8>, JsValue> {
    let msg_type = wire::MessageType::from_u16(msg_type);
    if matches!(msg_type, wire::MessageType::Unknown(_)) {
        return Err(JsValue::from_str("unknown wire message type code"));
    }
    let magic = product_magic(product)?;
    let frame = wire::Frame { msg_type, flags, request_id, body: body.to_vec() };
    frame.encode_as(magic).map_err(js_error)
}

/// Decode one complete wire frame (header plus body) with the upstream rules.
#[wasm_bindgen(js_name = frameDecode)]
pub fn frame_decode(bytes: &[u8]) -> std::result::Result<DecodedFrame, JsValue> {
    let frame = wire::Frame::decode(bytes).map_err(js_error)?;
    let product = std::str::from_utf8(&bytes[..4]).expect("upstream decode validated the magic").to_owned();
    Ok(DecodedFrame { msg_type: frame.msg_type, flags: frame.flags, request_id: frame.request_id, body: frame.body, product })
}

/// A decoded wire frame, as exposed to the host.
#[wasm_bindgen]
pub struct DecodedFrame {
    msg_type: wire::MessageType,
    flags: u16,
    request_id: u32,
    body: Vec<u8>,
    product: String,
}

#[wasm_bindgen]
impl DecodedFrame {
    /// Raw message type code.
    #[wasm_bindgen(getter, js_name = msgType)]
    pub fn msg_type(&self) -> u16 {
        self.msg_type.as_u16()
    }

    /// Reserved flags (v0 must be 0).
    #[wasm_bindgen(getter)]
    pub fn flags(&self) -> u16 {
        self.flags
    }

    /// Echoed request id.
    #[wasm_bindgen(getter, js_name = requestId)]
    pub fn request_id(&self) -> u32 {
        self.request_id
    }

    /// Payload bytes.
    #[wasm_bindgen(getter)]
    pub fn body(&self) -> Vec<u8> {
        self.body.clone()
    }

    /// Product magic the sender claimed (`"YYDB"` or `"YYDS"`).
    #[wasm_bindgen(getter)]
    pub fn product(&self) -> String {
        self.product.clone()
    }
}

/// Encode a `SchemaEnsure` body.
#[wasm_bindgen(js_name = encodeSchemaEnsure)]
pub fn encode_schema_ensure(version: u32, document: &str) -> Vec<u8> {
    wire::encode_schema_ensure(version, document)
}

/// Decode a `SchemaEnsure` body as JSON `{ version, document }`.
#[wasm_bindgen(js_name = decodeSchemaEnsure)]
pub fn decode_schema_ensure(body: &[u8]) -> std::result::Result<String, JsValue> {
    let (version, document) = wire::decode_schema_ensure(body).map_err(js_error)?;
    Ok(serde_json::json!({ "version": version, "document": document }).to_string())
}

/// Encode a `KvGet` body.
#[wasm_bindgen(js_name = encodeKvGet)]
pub fn encode_kv_get(key: &str) -> Vec<u8> {
    wire::encode_kv_get(key)
}

/// Decode a `KvGet` body (the key).
#[wasm_bindgen(js_name = decodeKvGet)]
pub fn decode_kv_get(body: &[u8]) -> std::result::Result<String, JsValue> {
    wire::decode_kv_get(body).map_err(js_error)
}

/// Encode a `KvPut` body.
#[wasm_bindgen(js_name = encodeKvPut)]
pub fn encode_kv_put(key: &str, value: &[u8]) -> Vec<u8> {
    wire::encode_kv_put(key, value)
}

/// Decode a `KvPut` body as JSON `{ key, value }` with `value` a byte array.
#[wasm_bindgen(js_name = decodeKvPut)]
pub fn decode_kv_put(body: &[u8]) -> std::result::Result<String, JsValue> {
    let (key, value) = wire::decode_kv_put(body).map_err(js_error)?;
    Ok(serde_json::json!({ "key": key, "value": value }).to_string())
}

/// Encode an `Authenticate` request body (opaque bearer token).
#[wasm_bindgen(js_name = encodeAuthenticate)]
pub fn encode_authenticate(token: &str) -> Vec<u8> {
    wire::encode_authenticate(token)
}

/// Decode an `AuthenticateOk` reply body (the session token).
#[wasm_bindgen(js_name = decodeAuthenticateOk)]
pub fn decode_authenticate_ok(body: &[u8]) -> std::result::Result<String, JsValue> {
    wire::decode_authenticate_ok(body).map_err(js_error)
}

/// Decode a `SchemaGetOk` body as JSON `{ version, document }`, or `null`.
#[wasm_bindgen(js_name = decodeSchemaGetOk)]
pub fn decode_schema_get_ok(body: &[u8]) -> std::result::Result<Option<String>, JsValue> {
    let schema = wire::decode_schema_get_ok(body).map_err(js_error)?;
    Ok(schema.map(|schema| serde_json::json!({ "version": schema.version, "document": schema.document }).to_string()))
}

/// Decode a `KvGetOk` body (the value bytes), or `null`.
#[wasm_bindgen(js_name = decodeKvGetOk)]
pub fn decode_kv_get_ok(body: &[u8]) -> std::result::Result<Option<Vec<u8>>, JsValue> {
    wire::decode_kv_get_ok(body).map_err(js_error)
}

/// Encode a `ScalarCall` body from JSON `{ name, version, args }`.
#[wasm_bindgen(js_name = encodeScalarCall)]
pub fn encode_scalar_call(request_json: &str) -> std::result::Result<Vec<u8>, JsValue> {
    let (name, version, args) = session::parse_scalar_call_json(request_json).map_err(js_error)?;
    wire::encode_scalar_call(&name, version, &args).map_err(js_error)
}

/// Decode a `ScalarCallOk` body as JSON (host-tagged scalars included).
#[wasm_bindgen(js_name = decodeScalarCallOk)]
pub fn decode_scalar_call_ok(body: &[u8]) -> std::result::Result<String, JsValue> {
    let value = wire::decode_scalar_call_ok(body).map_err(js_error)?;
    session::value_to_json(value).map(|json| json.to_string()).map_err(js_error)
}

/// Encode a `MicroRegister` body from the JSON descriptor shape used by
/// `registerMicroJson`.
#[wasm_bindgen(js_name = encodeMicroRegister)]
pub fn encode_micro_register(payload_json: &str) -> std::result::Result<Vec<u8>, JsValue> {
    let definition = session::parse_micro_register_json(payload_json).map_err(js_error)?;
    wire::encode_micro_register(&definition).map_err(js_error)
}

/// Decode a `MicroRegister` body as the `registerMicroJson` descriptor shape.
#[wasm_bindgen(js_name = decodeMicroRegister)]
pub fn decode_micro_register(body: &[u8]) -> std::result::Result<String, JsValue> {
    let definition = wire::decode_micro_register(body).map_err(js_error)?;
    let fingerprint: Vec<u8> = definition.fingerprint.to_vec();
    Ok(serde_json::json!({
        "name": definition.identity.name(),
        "version": definition.identity.version,
        "argTypes": definition.signature.args.iter().copied().map(udf_type_tag).collect::<Vec<_>>(),
        "returnType": udf_type_tag(definition.signature.returns),
        "hostId": definition.handle.host_id,
        "functionId": definition.handle.function_id,
        "handleVersion": definition.handle.version,
        "fingerprint": fingerprint,
    })
    .to_string())
}

/// Decode a `MicroHostInvoke` body as JSON
/// `{ hostId, handleVersion, functionId, args }`.
#[wasm_bindgen(js_name = decodeMicroHostInvoke)]
pub fn decode_micro_host_invoke(body: &[u8]) -> std::result::Result<String, JsValue> {
    let (handle, args) = wire::decode_micro_host_invoke(body).map_err(js_error)?;
    Ok(serde_json::json!({
        "hostId": handle.host_id,
        "handleVersion": handle.version,
        "functionId": handle.function_id,
        "args": args.iter().map(udf_value_json).collect::<Vec<_>>(),
    })
    .to_string())
}

/// Encode a `MicroHostInvokeOk` body from a JSON scalar value.
#[wasm_bindgen(js_name = encodeMicroHostInvokeOk)]
pub fn encode_micro_host_invoke_ok(value_json: &str) -> std::result::Result<Vec<u8>, JsValue> {
    let parsed: serde_json::Value = serde_json::from_str(value_json)
        .map_err(|error| JsValue::from_str(&format!("wire udf value json is invalid: {error}")))?;
    let value = udf_value_from_json(&parsed)?;
    Ok(wire::encode_micro_host_invoke_ok(value))
}

/// Encode an `Error` frame body from a diagnostic envelope JSON.
#[wasm_bindgen(js_name = encodeErrorBody)]
pub fn encode_error_body(diagnostic_json: &str) -> std::result::Result<Vec<u8>, JsValue> {
    let envelope: DiagnosticEnvelope = serde_json::from_str(diagnostic_json)
        .map_err(|error| JsValue::from_str(&format!("wire error body is not a valid diagnostic envelope: {error}")))?;
    serde_json::to_vec(&envelope).map_err(|error| JsValue::from_str(&error.to_string()))
}

/// Decode an `Error` frame body as the diagnostic JSON, validated against the
/// upstream envelope contract.
#[wasm_bindgen(js_name = decodeErrorBody)]
pub fn decode_error_body(body: &[u8]) -> std::result::Result<String, JsValue> {
    let error = wire::decode_error(body).map_err(js_error)?;
    let mut json = serde_json::json!({
        "code": error.code,
        "message": error.message,
        "operation": error.operation,
        "retryable": error.retryable,
    });
    if let Some(params) = error.params {
        json["params"] = serde_json::Value::Object(
            params.into_iter().map(|(key, value)| (key, serde_json::Value::String(value))).collect(),
        );
    }
    Ok(json.to_string())
}
