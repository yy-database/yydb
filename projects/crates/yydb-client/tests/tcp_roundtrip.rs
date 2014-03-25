use std::{
    net::TcpListener,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use yydb::{wire::dispatch, Connection, Value};
use yydb_client::{Client, Error};

#[test]
fn connect_rejects_empty_endpoint() {
    assert!(matches!(Client::connect("  "), Err(Error::Unsupported(_))));
}

#[test]
fn tcp_roundtrip_against_local_dispatch_server() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let conn = Arc::new(Connection::open_in_memory().unwrap());
    conn.ensure_schema("table T { @@id: uuid }").unwrap();

    let server_conn = Arc::clone(&conn);
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        loop {
            let request = match yydb::wire::read_frame(&mut stream) {
                Ok(frame) => frame,
                Err(_) => break,
            };
            let response = dispatch(&server_conn, &request);
            yydb::wire::write_frame(&mut stream, &response).unwrap();
        }
    });

    let deadline = Instant::now() + Duration::from_secs(2);
    let client = loop {
        match Client::connect(addr.to_string()) {
            Ok(client) => break client,
            Err(_) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("{error}"),
        }
    };

    assert!(!client.server_version().unwrap().is_empty());
    client.put("k", b"v").unwrap();
    assert_eq!(client.get("k").unwrap().as_deref(), Some(b"v".as_slice()));
    let schema = client.schema().unwrap().unwrap();
    assert_eq!(schema.version, 1);

    conn.create_scalar("double", 1, |args| match args {
        [Value::I64(n)] => Ok(Value::I64(n * 2)),
        _ => Err(Error::Udf {
            name: "double".into(),
            message: "expected i64".into(),
        }),
    })
    .unwrap();
    assert_eq!(
        client
            .call_scalar_version("double", 1, &[Value::I64(21)])
            .unwrap(),
        Value::I64(42)
    );
}
