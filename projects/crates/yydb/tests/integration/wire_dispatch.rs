use yydb::{
    wire::{
        decode_error, decode_kv_get_ok, decode_micro_host_invoke, decode_micro_host_invoke_ok,
        decode_scalar_call_ok, encode_kv_get, encode_micro_host_invoke,
        encode_micro_host_invoke_ok, encode_micro_register, encode_scalar_call,
        encode_schema_ensure, Frame, MessageType, MAGIC_YYDB, MAGIC_YYDS, WIRE_VERSION,
    },
    Connection, HostFunctionHandle, UdfValue, Value,
};
use yydb_server::{dispatch, is_loopback_host};

#[test]
fn roundtrip_frame() {
    let frame = Frame::new(MessageType::Hello, 7, b"hi".to_vec());
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
    conn.ensure_schema("table T { @@id: uuid }").unwrap();
    conn.put("a", b"b").unwrap();

    let hello = dispatch(&conn, &Frame::new(MessageType::Hello, 1, Vec::new()));
    assert_eq!(hello.msg_type, MessageType::HelloOk);
    let body = std::str::from_utf8(&hello.body).expect("hello body utf8");
    assert!(
        !body.is_empty()
            && body
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b'.'),
        "hello ok body should be a semver string, got {body:?}"
    );

    let info = dispatch(&conn, &Frame::new(MessageType::Info, 2, Vec::new()));
    assert_eq!(info.msg_type, MessageType::InfoOk);
    let text = String::from_utf8(info.body).unwrap();
    assert!(text.contains("records=1"));

    let get = dispatch(
        &conn,
        &Frame::new(MessageType::KvGet, 3, encode_kv_get("a")),
    );
    assert_eq!(get.msg_type, MessageType::KvGetOk);
    assert_eq!(
        decode_kv_get_ok(&get.body).unwrap().as_deref(),
        Some(b"b".as_slice())
    );
}

#[test]
fn dispatch_micro_register_records_session_metadata() {
    use yydb_udf::HostMicroDefinition;

    let conn = Connection::open_in_memory().unwrap();
    let definition = HostMicroDefinition::from_scalar(
        "text.normalize",
        1,
        vec![yydb_udf::UdfType::Text],
        yydb_udf::UdfType::Text,
        9,
        "normalize",
        1,
        [2u8; 32],
    )
    .unwrap();
    let response = dispatch(
        &conn,
        &Frame::new(
            MessageType::MicroRegister,
            4,
            encode_micro_register(&definition),
        ),
    );
    assert_eq!(response.msg_type, MessageType::MicroRegisterOk);
    assert!(conn.list_scalars().contains(&"text.normalize".to_owned()));
}

#[test]
fn micro_host_invoke_roundtrip_codec() {
    let handle = HostFunctionHandle {
        host_id: 9,
        function_id: "normalize".into(),
        version: 1,
    };
    let args = vec![UdfValue::Text("  Hi  ".into())];
    let body = encode_micro_host_invoke(&handle, &args);
    let (decoded_handle, decoded_args) = decode_micro_host_invoke(&body).unwrap();
    assert_eq!(decoded_handle, handle);
    assert_eq!(decoded_args, args);
    let ok = encode_micro_host_invoke_ok(UdfValue::Text("hi".into()));
    assert_eq!(
        decode_micro_host_invoke_ok(&ok).unwrap(),
        UdfValue::Text("hi".into())
    );
}

#[test]
fn dispatch_scalar_call_routes_through_registry() {
    let conn = Connection::open_in_memory().unwrap();
    conn.create_scalar("double", 1, |args| match args {
        [Value::I64(n)] => Ok(Value::I64(n * 2)),
        _ => Err(yydb::Error::Udf {
            name: "double".into(),
            message: "expected i64".into(),
        }),
    })
    .unwrap();
    let body = encode_scalar_call("double", 1, &[Value::I64(21)]).unwrap();
    let response = dispatch(&conn, &Frame::new(MessageType::ScalarCall, 5, body));
    assert_eq!(response.msg_type, MessageType::ScalarCallOk);
    assert_eq!(
        decode_scalar_call_ok(&response.body).unwrap(),
        Value::I64(42)
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

#[test]
fn error_frame_carries_structured_code_and_message() {
    let conn = Connection::open_in_memory().unwrap();
    let response = dispatch(
        &conn,
        &Frame::new(
            MessageType::SchemaEnsure,
            6,
            encode_schema_ensure(1, "table {"),
        ),
    );
    assert_eq!(response.msg_type, MessageType::Error);
    let error = decode_error(&response.body).expect("structured error body");
    assert_eq!(error.code, "yydb.schema.invalid");
    assert!(!error.message.is_empty());
}

#[test]
fn error_frame_codes_cover_protocol_and_corrupt_paths() {
    let conn = Connection::open_in_memory().unwrap();

    let unknown = dispatch(
        &conn,
        &Frame::new(MessageType::Unknown(9999), 7, Vec::new()),
    );
    assert_eq!(unknown.msg_type, MessageType::Error);
    assert_eq!(decode_error(&unknown.body).unwrap().code, "yydb.protocol");

    let truncated = dispatch(&conn, &Frame::new(MessageType::KvGet, 8, vec![1, 0]));
    assert_eq!(truncated.msg_type, MessageType::Error);
    assert_eq!(decode_error(&truncated.body).unwrap().code, "yydb.corrupt");
}

#[test]
fn error_code_taxonomy_is_stable() {
    assert_eq!(yydb::Error::SessionClosed.code(), "yydb.session.closed");
    assert_eq!(
        yydb::Error::Schema {
            message: "bad".into()
        }
        .code(),
        "yydb.schema.invalid"
    );
    assert_eq!(
        yydb::Error::Remote {
            code: "yydb.remote.probe".into(),
            message: "peer".into(),
        }
        .code(),
        "yydb.remote.probe"
    );
}
