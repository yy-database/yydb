use std::collections::BTreeMap;

use yydb::{Connection, Value};

const USER_SCHEMA: &str = r#"
table User {
    @@user_id: uuid,
    @user_name: utf8,
    active: bool,
}
"#;

fn user_row(user_name: &str, active: bool) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("user_name".into(), Value::Text(user_name.into())),
        ("active".into(), Value::Bool(active)),
    ])
}

#[test]
fn query_filters_active_users() {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema(1, USER_SCHEMA).expect("schema");
    conn.upsert_row("User", "ada", user_row("ada", true))
        .expect("seed ada");
    conn.upsert_row("User", "linus", user_row("linus", true))
        .expect("seed linus");
    conn.upsert_row("User", "grace", user_row("grace", false))
        .expect("seed grace");

    let rows = conn
        .query(r#"User.filter(x => x.active).collect()"#)
        .expect("query");
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.get("active") == Some(&Value::Bool(true))));
}

#[test]
fn query_collect_all_rows_with_true_predicate() {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema(1, USER_SCHEMA).expect("schema");
    conn.upsert_row("User", "ada", user_row("ada", true))
        .expect("seed");

    let rows = conn
        .query(r#"User.filter(x => true).collect()"#)
        .expect("query");
    assert_eq!(rows.len(), 1);
}

#[test]
fn query_rows_survive_reopen() {
    let dir = std::env::temp_dir().join(format!(
        "yydb-vos-query-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("db.yydb");

    {
        let conn = Connection::open(&path).expect("open");
        conn.ensure_schema(1, USER_SCHEMA).expect("schema");
        conn.upsert_row("User", "ada", user_row("ada", true))
            .expect("seed");
    }

    let conn = Connection::open(&path).expect("reopen");
    let rows = conn
        .query(r#"User.filter(x => true).collect()"#)
        .expect("query");
    assert_eq!(rows.len(), 1);
    let _ = std::fs::remove_file(&path);
}
