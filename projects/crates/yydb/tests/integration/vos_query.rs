use yydb::{Connection, Value};

const USER_SCHEMA: &str = r#"
table User {
    @@user_id: uuid,
    user_name: utf8,
    active: bool,
}
"#;

fn insert_ada(conn: &Connection) {
    conn.execute_vos_write(
        r#"User::insert({
            user_id: "550e8400-e29b-41d4-a716-446655440000",
            user_name: "ada",
            active: true,
        })"#,
    )
    .expect("insert ada");
}

#[test]
fn query_vos_reads_rows_after_insert() {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema(USER_SCHEMA).expect("schema");
    insert_ada(&conn);

    let rows = conn.query_vos("User.all().collect()").expect("query");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("user_name"), Some(&Value::Text("ada".into())));
}

#[test]
fn execute_vos_write_participates_in_transactions() {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema(USER_SCHEMA).expect("schema");

    conn.begin().expect("begin");
    insert_ada(&conn);
    assert_eq!(conn.query_vos("User.all().collect()").unwrap().len(), 1);
    conn.rollback().expect("rollback");
    assert_eq!(conn.query_vos("User.all().collect()").unwrap().len(), 0);

    conn.begin().expect("begin");
    insert_ada(&conn);
    conn.commit().expect("commit");
    assert_eq!(conn.query_vos("User.all().collect()").unwrap().len(), 1);
}

#[test]
fn query_vos_requires_schema() {
    let conn = Connection::open_in_memory().expect("open");
    let err = conn
        .query_vos("User.all().collect()")
        .expect_err("no schema");
    assert!(err.to_string().contains("ensure_schema"));
}
