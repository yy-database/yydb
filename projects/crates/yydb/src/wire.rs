//! YY wire protocol for VOS-native serve.
//!
//! Header prefix: product magic `YYDB` | `YYDS` (backend **self-claim** only —
//! same wire semantics either way), then four ASCII version digits (`0000`,
//! `0001`, …). Frontends accept either magic and do not require it to match a
//! preferred product. This crate encodes as `YYDB`. See `documentation/serve-protocol.md`.

use std::io::{Read, Write};

use yydb_types::{Error, Result, SchemaVersion};

use crate::Connection;

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
pub enum MsgType {
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
    /// Error reply.
    Error,
    /// Unrecognized type (still framed so the peer can answer with Error).
    Unknown(u16),
}

impl MsgType {
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
            Self::Error => 255,
            Self::Unknown(code) => code,
        }
    }
}

/// One length-prefixed binary frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Message type.
    pub msg_type: MsgType,
    /// Reserved flags (must be 0 in v1).
    pub flags: u16,
    /// Client-chosen request id (echoed on replies).
    pub request_id: u32,
    /// Payload bytes.
    pub body: Vec<u8>,
}

impl Frame {
    /// Build a frame with empty flags.
    pub fn new(msg_type: MsgType, request_id: u32, body: impl Into<Vec<u8>>) -> Self {
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
        let msg_type = MsgType::from_u16(msg_raw);
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

fn error_frame(request_id: u32, message: impl AsRef<str>) -> Frame {
    Frame::new(
        MsgType::Error,
        request_id,
        message.as_ref().as_bytes().to_vec(),
    )
}

fn encode_schema_ok(schema: Option<SchemaVersion>) -> Vec<u8> {
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

fn decode_schema_ensure(body: &[u8]) -> Result<(u32, String)> {
    let mut offset = 0;
    let version = read_u32(body, &mut offset)?;
    let doc_len = read_u32(body, &mut offset)?;
    let doc = read_bytes(body, &mut offset, doc_len)?;
    let document = std::str::from_utf8(doc)
        .map_err(|_| Error::Corrupt("schema document is not utf-8"))?
        .to_owned();
    Ok((version, document))
}

fn decode_kv_get(body: &[u8]) -> Result<String> {
    let mut offset = 0;
    let key_len = read_u32(body, &mut offset)?;
    let key = read_bytes(body, &mut offset, key_len)?;
    Ok(std::str::from_utf8(key)
        .map_err(|_| Error::Corrupt("kv key is not utf-8"))?
        .to_owned())
}

fn decode_kv_put(body: &[u8]) -> Result<(String, Vec<u8>)> {
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

fn encode_kv_get_ok(value: Option<Vec<u8>>) -> Vec<u8> {
    match value {
        None => vec![0],
        Some(bytes) => {
            let mut body = vec![1];
            push_bytes(&mut body, &bytes);
            body
        }
    }
}

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
        MsgType::Hello => Frame::new(MsgType::HelloOk, id, crate::version().as_bytes().to_vec()),
        MsgType::Info => match info_body(conn) {
            Ok(body) => Frame::new(MsgType::InfoOk, id, body),
            Err(error) => error_frame(id, error.to_string()),
        },
        MsgType::SchemaGet => match conn.schema() {
            Ok(schema) => Frame::new(MsgType::SchemaGetOk, id, encode_schema_ok(schema)),
            Err(error) => error_frame(id, error.to_string()),
        },
        MsgType::SchemaEnsure => match decode_schema_ensure(&request.body) {
            Ok((version, document)) => match conn.ensure_schema(version, &document) {
                Ok(()) => Frame::new(MsgType::SchemaEnsureOk, id, Vec::new()),
                Err(error) => error_frame(id, error.to_string()),
            },
            Err(error) => error_frame(id, error.to_string()),
        },
        MsgType::KvGet => match decode_kv_get(&request.body) {
            Ok(key) => match conn.get(&key) {
                Ok(value) => Frame::new(MsgType::KvGetOk, id, encode_kv_get_ok(value)),
                Err(error) => error_frame(id, error.to_string()),
            },
            Err(error) => error_frame(id, error.to_string()),
        },
        MsgType::KvPut => match decode_kv_put(&request.body) {
            Ok((key, value)) => match conn.put(key, value) {
                Ok(()) => Frame::new(MsgType::KvPutOk, id, Vec::new()),
                Err(error) => error_frame(id, error.to_string()),
            },
            Err(error) => error_frame(id, error.to_string()),
        },
        MsgType::HelloOk
        | MsgType::InfoOk
        | MsgType::SchemaGetOk
        | MsgType::SchemaEnsureOk
        | MsgType::KvGetOk
        | MsgType::KvPutOk
        | MsgType::Error => error_frame(id, "unexpected client message type"),
        MsgType::Unknown(code) => error_frame(id, format!("unknown msg_type {code}")),
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

/// Decode an `Error` body as UTF-8.
pub fn decode_error_message(body: &[u8]) -> String {
    String::from_utf8_lossy(body).into_owned()
}

/// Whether a `--bind` host is loopback (product-safe default).
pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim().trim_start_matches('[').trim_end_matches(']');
    matches!(
        host.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "::1" | "localhost" | "0:0:0:0:0:0:0:1"
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Connection;

    #[test]
    fn roundtrip_frame() {
        let frame = Frame::new(MsgType::Hello, 7, b"hi".to_vec());
        let encoded = frame.encode().unwrap();
        assert_eq!(&encoded[..4], MAGIC_YYDB);
        assert_eq!(&encoded[4..8], WIRE_VERSION);
        let decoded = Frame::decode(&encoded).unwrap();
        assert_eq!(decoded, frame);

        let as_yyds = frame.encode_as(MAGIC_YYDS).unwrap();
        assert_eq!(&as_yyds[..4], MAGIC_YYDS);
        assert_eq!(Frame::decode(&as_yyds).unwrap(), frame);
    }

    #[test]
    fn dispatch_hello_info_kv() {
        let conn = Connection::open_in_memory().unwrap();
        conn.ensure_schema(1, "table T { @@id: uuid }").unwrap();
        conn.put("a", b"b").unwrap();

        let hello = dispatch(&conn, &Frame::new(MsgType::Hello, 1, Vec::new()));
        assert_eq!(hello.msg_type, MsgType::HelloOk);
        assert_eq!(hello.body, crate::version().as_bytes());

        let info = dispatch(&conn, &Frame::new(MsgType::Info, 2, Vec::new()));
        assert_eq!(info.msg_type, MsgType::InfoOk);
        let text = String::from_utf8(info.body).unwrap();
        assert!(text.contains("records=1"));

        let get = dispatch(&conn, &Frame::new(MsgType::KvGet, 3, encode_kv_get("a")));
        assert_eq!(get.msg_type, MsgType::KvGetOk);
        assert_eq!(
            decode_kv_get_ok(&get.body).unwrap().as_deref(),
            Some(b"b".as_slice())
        );
    }

    #[test]
    fn loopback_host_detection() {
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("::1"));
        assert!(!is_loopback_host("0.0.0.0"));
        assert!(!is_loopback_host("192.168.1.1"));
    }
}
