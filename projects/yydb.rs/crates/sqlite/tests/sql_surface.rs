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
        conn.execute_with_params(
            "INSERT INTO Account (account_id, email, score) VALUES (?, ?, ?)",
            &[Value::I64(3), Value::Text("grace@example.test".into()), Value::I64(20)],
        )
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

// ─── Transaction edge cases ────────────────────────────────────────────

#[test]
fn transaction_in_progress_rejects_second_begin() {
    let conn = SqliteConnection::open_in_memory().expect("open");
    conn.ensure_schema(SCHEMA).expect("schema");

    conn.begin().expect("first begin");
    let err = conn.begin().expect_err("second begin must fail");
    assert!(format!("{err}").contains("already open") || format!("{err}").contains("transaction"));
    conn.rollback().expect("cleanup");
}

#[test]
fn commit_without_open_transaction_errors() {
    let conn = SqliteConnection::open_in_memory().expect("open");
    let err = conn.commit().expect_err("commit without txn must fail");
    assert!(format!("{err}").contains("no open transaction") || format!("{err}").contains("transaction"));
}

#[test]
fn rollback_without_open_transaction_errors() {
    let conn = SqliteConnection::open_in_memory().expect("open");
    let err = conn.rollback().expect_err("rollback without txn must fail");
    assert!(format!("{err}").contains("no open transaction") || format!("{err}").contains("transaction"));
}

#[test]
fn transaction_isolation_visible_to_all_surfaces() {
    // VOS and SQL writes inside the same transaction must both be visible to
    // both surfaces within the transaction, and both disappear on rollback.
    let conn = SqliteConnection::open_in_memory().expect("open");
    conn.ensure_schema(SCHEMA).expect("schema");

    conn.begin().expect("begin");

    // SQL insert inside transaction
    conn.execute("INSERT INTO Account (account_id, email, score) VALUES (1, 'sql@test', 100)").expect("sql insert");
    assert_eq!(ids(&conn.query("SELECT account_id FROM Account").unwrap()), vec![1]);

    // VOS insert inside same transaction (must also see SQL's row)
    conn.connection()
        .execute_vos_write(r#"Account::insert({ account_id: 2, email: "vos@test", score: 200 })"#)
        .expect("vos insert");

    // Both surfaces see both rows
    assert_eq!(ids(&conn.query("SELECT account_id FROM Account ORDER BY account_id").unwrap()), vec![1, 2]);
    assert_eq!(conn.connection().query_vos("Account.all().collect()").unwrap().len(), 2);

    conn.rollback().expect("rollback");

    // After rollback, both surfaces see empty
    assert!(conn.query("SELECT account_id FROM Account").unwrap().is_empty());
    assert!(conn.connection().query_vos("Account.all().collect()").unwrap().is_empty());
}

#[test]
fn file_backed_transaction_survives_reopen_after_commit() {
    let directory = std::env::temp_dir().join(format!("yydb-txn-reopen-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("temp dir");
    let path = directory.join("txn.yydb");

    {
        let conn = SqliteConnection::open(&path).expect("open");
        conn.ensure_schema(SCHEMA).expect("schema");

        conn.begin().expect("begin");
        conn.execute("INSERT INTO Account (account_id, email, score) VALUES (42, 'persist@test', 7)").expect("insert");
        // Not yet visible to a fresh connection — but we haven't committed yet.
        conn.commit().expect("commit");
    }

    // Reopen: committed transaction data persists.
    let conn = SqliteConnection::open(&path).expect("reopen");
    let rows = conn.query("SELECT account_id, score FROM Account WHERE account_id = 42").expect("reopen");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("score"), Some(&Value::I64(7)));

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn file_backed_transaction_vanishes_on_rollback() {
    let directory = std::env::temp_dir().join(format!("yydb-txn-rollback-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("temp dir");
    let path = directory.join("roll.yydb");

    {
        let conn = SqliteConnection::open(&path).expect("open");
        conn.ensure_schema(SCHEMA).expect("schema");

        conn.begin().expect("begin");
        conn.execute("INSERT INTO Account (account_id, email, score) VALUES (99, 'gone@test', 1)").expect("insert");
        conn.rollback().expect("rollback");
    }

    let conn = SqliteConnection::open(&path).expect("reopen");
    assert!(conn.query("SELECT account_id FROM Account").unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn mixed_sql_dml_within_single_transaction() {
    let conn = SqliteConnection::open_in_memory().expect("open");
    conn.ensure_schema(SCHEMA).expect("schema");

    // Populate outside transaction
    conn.execute("INSERT INTO Account (account_id, email, score) VALUES (1, 'a@t', 10)").expect("seed 1");
    conn.execute("INSERT INTO Account (account_id, email, score) VALUES (2, 'b@t', 20)").expect("seed 2");

    conn.begin().expect("begin");

    // INSERT inside txn
    conn.execute("INSERT INTO Account (account_id, email, score) VALUES (3, 'c@t', 30)").expect("insert in txn");
    // UPDATE inside txn (touches pre-existing row)
    conn.execute("UPDATE Account SET score = 500 WHERE account_id = 1").expect("update in txn");
    // DELETE inside txn (removes pre-existing row)
    conn.execute("DELETE FROM Account WHERE account_id = 2").expect("delete in txn");

    // All changes visible within txn
    let snapshot = ids(&conn.query("SELECT account_id FROM Account ORDER BY account_id").unwrap());
    assert_eq!(snapshot, vec![1, 3]);
    assert_eq!(
        conn.query("SELECT score FROM Account WHERE account_id = 1").unwrap()[0].get("score"),
        Some(&Value::I64(500))
    );

    conn.rollback().expect("rollback");

    // Back to original state
    assert_eq!(ids(&conn.query("SELECT account_id FROM Account ORDER BY account_id").unwrap()), vec![1, 2]);
    assert_eq!(
        conn.query("SELECT score FROM Account WHERE account_id = 1").unwrap()[0].get("score"),
        Some(&Value::I64(10))
    );
}
