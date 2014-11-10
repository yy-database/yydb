use yydb::Connection;

#[test]
fn begin_rollback_toggles_transaction_state() {
    let conn = Connection::open_in_memory().expect("open");
    assert!(!conn.in_transaction());
    conn.begin().expect("begin");
    assert!(conn.in_transaction());
    conn.rollback().expect("rollback");
    assert!(!conn.in_transaction());
}

#[test]
fn nested_begin_is_rejected() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");
    let err = conn.begin().expect_err("nested begin");
    assert!(err.to_string().contains("transaction already open"));
    conn.rollback().expect("rollback");
}

#[test]
fn commit_without_open_transaction_errors() {
    let conn = Connection::open_in_memory().expect("open");
    let err = conn.commit().expect_err("commit without txn");
    assert!(err.to_string().contains("no open transaction"));
}

#[test]
fn rollback_discards_kv_writes() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");
    conn.put("key", b"value").expect("put");
    assert_eq!(conn.get("key").expect("read snapshot"), Some(b"value".to_vec()));
    conn.rollback().expect("rollback");
    assert_eq!(conn.get("key").expect("read after rollback"), None);
}

#[test]
fn commit_persists_kv_writes() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");
    conn.put("key", b"value").expect("put");
    conn.commit().expect("commit");
    assert_eq!(conn.get("key").expect("read after commit"), Some(b"value".to_vec()));
}
