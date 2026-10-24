//! SQL frontend on the connection surface: the same rows VOS writes, addressed
//! with SQL statements.

use yydb::{Connection, Value};

const ACCOUNT_SCHEMA: &str = r#"
table Account {
    @@account_id: i64,
    email: utf8,
    tenant: utf8,
    score: i64,
}
"#;

fn seed(conn: &Connection) {
    conn.execute_sql_write(
        "INSERT INTO Account (account_id, email, tenant, score) VALUES (?1, ?2, ?3, ?4)",
        &[Value::I64(1), Value::Text("ada@example.test".into()), Value::Text("t1".into()), Value::I64(10)],
    )
    .expect("insert ada");
    conn.execute_sql_write(
        "INSERT INTO Account (account_id, email, tenant, score) VALUES (?1, ?2, ?3, ?4)",
        &[Value::I64(2), Value::Text("linus@example.test".into()), Value::Text("t1".into()), Value::I64(30)],
    )
    .expect("insert linus");
}

fn account_ids(conn: &Connection, sql: &str) -> Vec<i64> {
    conn.query_sql(sql)
        .expect("query")
        .iter()
        .map(|row| match row.get("account_id") {
            Some(Value::I64(id)) => *id,
            other => panic!("unexpected account_id {other:?}"),
        })
        .collect()
}

#[test]
fn sql_insert_and_select_read_through_the_connection() {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema(ACCOUNT_SCHEMA).expect("schema");
    seed(&conn);

    assert_eq!(account_ids(&conn, "SELECT account_id FROM Account"), vec![1, 2]);
    assert_eq!(account_ids(&conn, "SELECT account_id FROM Account WHERE score > 10"), vec![2]);
    assert_eq!(account_ids(&conn, "SELECT account_id FROM Account ORDER BY score DESC"), vec![2, 1]);
}

#[test]
fn sql_select_binds_positional_parameters() {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema(ACCOUNT_SCHEMA).expect("schema");
    seed(&conn);

    let rows = conn
        .query_sql_with_params("SELECT email FROM Account WHERE account_id = ?", &[Value::I64(1)])
        .expect("parameterized select");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("email"), Some(&Value::Text("ada@example.test".into())));
}

#[test]
fn sql_update_and_delete_report_affected_rows() {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema(ACCOUNT_SCHEMA).expect("schema");
    seed(&conn);

    let updated = conn
        .execute_sql_write("UPDATE Account SET score = ? WHERE account_id = ?", &[Value::I64(99), Value::I64(1)])
        .expect("update");
    assert_eq!(updated, 1);
    assert_eq!(conn.query_sql("SELECT score FROM Account WHERE account_id = 1").unwrap()[0].get("score"), Some(&Value::I64(99)));

    let deleted = conn.execute_sql_write("DELETE FROM Account WHERE tenant = ?", &[Value::Text("t1".into())]).expect("delete");
    assert_eq!(deleted, 2);
    assert!(conn.query_sql("SELECT account_id FROM Account").unwrap().is_empty());
}

#[test]
fn sql_writes_participate_in_transactions() {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema(ACCOUNT_SCHEMA).expect("schema");

    conn.begin().expect("begin");
    seed(&conn);
    assert_eq!(account_ids(&conn, "SELECT account_id FROM Account"), vec![1, 2]);
    conn.rollback().expect("rollback");
    assert!(account_ids(&conn, "SELECT account_id FROM Account").is_empty());

    conn.begin().expect("begin");
    seed(&conn);
    conn.commit().expect("commit");
    assert_eq!(account_ids(&conn, "SELECT account_id FROM Account"), vec![1, 2]);
}

#[test]
fn sql_and_vos_address_the_same_rows() {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema(ACCOUNT_SCHEMA).expect("schema");
    seed(&conn);

    let vos_rows = conn.query_vos("Account.all().collect()").expect("vos query");
    assert_eq!(vos_rows.len(), 2);

    conn.execute_vos_write(
        r#"Account::insert({ account_id: 3, email: "grace@example.test", tenant: "t2", score: 20 })"#,
    )
    .expect("vos insert");
    assert_eq!(account_ids(&conn, "SELECT account_id FROM Account"), vec![1, 2, 3]);
}
