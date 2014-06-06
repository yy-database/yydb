use yydb::Connection;

#[test]
fn main_snapshot_roundtrip_preserves_kv_and_schema() {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema("table Setting { @@id: uuid, key: utf8, value: utf8 }")
        .expect("schema");
    conn.put("theme", b"dark").expect("put");

    let bytes = conn.main_snapshot().expect("snapshot");
    let reopened = Connection::open_in_memory().expect("reopen");
    reopened.load_main_snapshot(&bytes).expect("load");

    assert_eq!(reopened.get("theme").expect("get"), Some(b"dark".to_vec()));
    let schema = reopened.schema().expect("schema").expect("present");
    assert!(schema.document.contains("table Setting"));
}
