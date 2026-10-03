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
    conn.ensure_schema(USER_SCHEMA).expect("schema");
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
    conn.ensure_schema(USER_SCHEMA).expect("schema");
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
        conn.ensure_schema(USER_SCHEMA).expect("schema");
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

const BLOG_SCHEMA: &str = r#"
table User {
    @@user_id: uuid,
    user_name: utf8,
    active: bool,
}

table Post {
    @@post_id: uuid,
    author: &User,
    title: utf8,
    published: bool,
}
"#;

#[test]
fn query_filters_posts_by_referenced_author_name() {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema(BLOG_SCHEMA).expect("schema");
    conn.execute(
        r#"
        User {
            user_id: "550e8400-e29b-41d4-a716-446655440000",
            user_name: "ada",
            active: true,
        }.insert()
        User {
            user_id: "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
            user_name: "linus",
            active: true,
        }.insert()
        Post {
            post_id: "11111111-1111-4111-8111-111111111101",
            author: "550e8400-e29b-41d4-a716-446655440000",
            title: "Ada post",
            published: true,
        }.insert()
        Post {
            post_id: "22222222-2222-4222-8222-222222222202",
            author: "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
            title: "Linus post",
            published: true,
        }.insert()
        "#,
    )
    .expect("seed");

    let rows = conn
        .query(
            r#"Post.filter(x => x.published && x.author.user_name == "ada").collect()"#,
        )
        .expect("query");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get("title"),
        Some(&Value::Text("Ada post".into()))
    );
}

#[test]
fn query_projects_nested_reference_fields() {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema(BLOG_SCHEMA).expect("schema");
    conn.execute(
        r#"
        User {
            user_id: "550e8400-e29b-41d4-a716-446655440000",
            user_name: "ada",
            active: true,
        }.insert()
        Post {
            post_id: "11111111-1111-4111-8111-111111111101",
            author: "550e8400-e29b-41d4-a716-446655440000",
            title: "Ada post",
            published: true,
        }.insert()
        "#,
    )
    .expect("seed");

    let rows = conn
        .query(
            r#"Post.filter(x => x.published).map(x => x.{ post_id, title, author: x.author.{ user_name } }).collect()"#,
        )
        .expect("query");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get("title"),
        Some(&Value::Text("Ada post".into()))
    );
    let author = rows[0]
        .get("author")
        .expect("nested author projection");
    match author {
        Value::Row(nested) => {
            assert_eq!(
                nested.get("user_name"),
                Some(&Value::Text("ada".into()))
            );
        }
        other => panic!("expected nested row, got {other:?}"),
    }
}
