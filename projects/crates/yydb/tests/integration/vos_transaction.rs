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
