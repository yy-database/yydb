//! Writable SQLite dialect surface: SQL statements executed by YYDB operators.

use sqlite::SqliteConnection;
use yydb::Value;

const SCHEMA: &str = r#"
table Account {
    @@account_id: i64,
    email: utf8,
    score: i64,
}
"#;

fn ids(rows: &[std::collections::BTreeMap<String, Value>]) -> Vec<i64> {
    rows.iter()
        .map(|row| match row.get("account_id") {
            Some(Value::I64(id)) => *id,
            other => panic!("unexpected account_id {other:?}"),
        })
        .collect()
}

#[test]
fn sql_round_trip_on_a_file_backed_database() {
    let directory = std::env::temp_dir().join(format!("yydb-sqlite-surface-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("temp directory");
    let path = directory.join("app.yydb");

    {
        let conn = SqliteConnection::open(&path).expect("open");
        conn.ensure_schema(SCHEMA).expect("schema");
        conn.execute("INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@example.test', 10)").expect("insert");
        conn.execute("INSERT INTO Account (account_id, email, score) VALUES (2, 'linus@example.test', 30)").expect("insert");
        conn.execute_with_params("INSERT INTO Account (account_id, email, score) VALUES (?, ?, ?)", &[
            Value::I64(3),
            Value::Text("grace@example.test".into()),
            Value::I64(20),
        ])
        .expect("parameterized insert");
        assert_eq!(ids(&conn.query("SELECT account_id FROM Account ORDER BY score DESC").unwrap()), vec![2, 3, 1]);
        conn.execute_with_params("UPDATE Account SET score = ? WHERE account_id = ?", &[Value::I64(99), Value::I64(1)])
            .expect("update");
        conn.execute("DELETE FROM Account WHERE score < 25").expect("delete");
        assert_eq!(ids(&conn.query("SELECT account_id FROM Account").unwrap()), vec![1, 2]);
    }

    // Reopen the same file: SQL sees the committed rows.
    let conn = SqliteConnection::open(&path).expect("reopen");
    let rows = conn.query("SELECT score FROM Account WHERE account_id = 1").expect("reopen query");
    assert_eq!(rows[0].get("score"), Some(&Value::I64(99)));

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn transactions_roll_back_sql_writes() {
    let conn = SqliteConnection::open_in_memory().expect("open");
    conn.ensure_schema(SCHEMA).expect("schema");

    conn.begin().expect("begin");
    conn.execute("INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@example.test', 10)").expect("insert");
    conn.rollback().expect("rollback");
    assert!(conn.query("SELECT account_id FROM Account").unwrap().is_empty());

    conn.begin().expect("begin");
    conn.execute("INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@example.test', 10)").expect("insert");
    conn.commit().expect("commit");
    assert_eq!(ids(&conn.query("SELECT account_id FROM Account").unwrap()), vec![1]);
}

#[test]
fn sql_and_vos_share_one_database() {
    let conn = SqliteConnection::open_in_memory().expect("open");
    conn.ensure_schema(SCHEMA).expect("schema");

    conn.connection()
        .execute_vos_write(r#"Account::insert({ account_id: 7, email: "vos@example.test", score: 1 })"#)
        .expect("vos insert");
    conn.execute("INSERT INTO Account (account_id, email, score) VALUES (8, 'sql@example.test', 2)").expect("sql insert");

    assert_eq!(conn.connection().query_vos("Account.all().collect()").unwrap().len(), 2);
    assert_eq!(ids(&conn.query("SELECT account_id FROM Account ORDER BY account_id").unwrap()), vec![7, 8]);
}

#[test]
fn invalid_sql_is_rejected_without_mutation() {
    let conn = SqliteConnection::open_in_memory().expect("open");
    conn.ensure_schema(SCHEMA).expect("schema");

    assert!(conn.query("SELECT * FROM Account WHERE email LIKE '%x%'").is_err());
    assert!(conn.execute("CREATE TABLE Other (id INTEGER)").is_err());
    assert!(conn.query("SELECT account_id FROM Account").unwrap().is_empty());
}
