use yydb::Connection;

const USER_SCHEMA: &str = r#"
table User {
    @@user_id: uuid,
    user_name: utf8,
    active: bool,
}
"#;

const INSERT_ADA: &str = r#"
User {
    user_id: "550e8400-e29b-41d4-a716-446655440000",
    user_name: "ada",
    active: true,
}.insert()
"#;

#[test]
fn transaction_commit_persists_execute_writes() {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema(1, USER_SCHEMA).expect("schema");
    conn.begin().expect("begin");
    assert!(conn.in_transaction());
    conn.execute(INSERT_ADA).expect("insert in txn");

    let hidden = conn
        .query(r#"User.filter(x => true).collect()"#)
        .expect("query in txn");
    assert_eq!(hidden.len(), 1);

    conn.commit().expect("commit");
    assert!(!conn.in_transaction());

    let rows = conn
        .query(r#"User.filter(x => true).collect()"#)
        .expect("query after commit");
    assert_eq!(rows.len(), 1);
}

#[test]
fn transaction_rollback_discards_execute_writes() {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema(1, USER_SCHEMA).expect("schema");
    conn.begin().expect("begin");
    conn.execute(INSERT_ADA).expect("insert in txn");
    conn.rollback().expect("rollback");
    assert!(!conn.in_transaction());

    let rows = conn
        .query(r#"User.filter(x => true).collect()"#)
        .expect("query after rollback");
    assert!(rows.is_empty());
}

#[test]
fn nested_begin_is_rejected() {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema(1, USER_SCHEMA).expect("schema");
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
