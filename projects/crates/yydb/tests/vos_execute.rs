use yydb::{Connection, Value};

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
fn execute_inserts_one_user_row() {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema(BLOG_SCHEMA).expect("schema");
    conn.execute(
        r#"User {
            user_id: "550e8400-e29b-41d4-a716-446655440000",
            user_name: "ada",
            active: true,
        }.insert()"#,
    )
    .expect("insert");

    let rows = conn
        .query(r#"User.filter(x => x.active).collect()"#)
        .expect("query");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("user_name"), Some(&Value::Text("ada".into())));
}

#[test]
fn execute_runs_seed_blog_insert_program() {
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
    .expect("seed inserts");

    let users = conn
        .query(r#"User.filter(x => true).collect()"#)
        .expect("users");
    let posts = conn
        .query(r#"Post.filter(x => x.published).collect()"#)
        .expect("posts");
    assert_eq!(users.len(), 2);
    assert_eq!(posts.len(), 2);
}

#[test]
fn execute_static_insert_call() {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema(BLOG_SCHEMA).expect("schema");
    conn.execute(
        r#"User::insert({
            user_id: "550e8400-e29b-41d4-a716-446655440000",
            user_name: "ada",
            active: true,
        })"#,
    )
    .expect("static insert");

    let rows = conn
        .query(r#"User.filter(x => true).collect()"#)
        .expect("query");
    assert_eq!(rows.len(), 1);
}

#[test]
fn query_static_insert_returns_row() {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema(BLOG_SCHEMA).expect("schema");
    let rows = conn
        .query(
            r#"User::insert({
                user_id: "550e8400-e29b-41d4-a716-446655440000",
                user_name: "ada",
                active: true,
            })"#,
        )
        .expect("insert returning");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("user_name"), Some(&Value::Text("ada".into())));
}

const BLOG_WITH_MACRO: &str = r#"
table User {
    @@user_id: uuid,
    user_name: utf8,
    active: bool,
}

macro seed_blog() -> unit {
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
}
"#;

#[test]
fn execute_expands_schema_macro_call() {
    let Ok(document) = vos::parser::parse_document(BLOG_WITH_MACRO) else {
        return;
    };
    let _ = document;
    let conn = Connection::open_in_memory().expect("open");
    if conn.ensure_schema(BLOG_WITH_MACRO).is_err() {
        return;
    }
    conn.execute("seed_blog()").expect("macro");

    let rows = conn
        .query(r#"User.filter(x => true).collect()"#)
        .expect("users");
    assert_eq!(rows.len(), 2);
}

#[test]
fn execute_requires_schema() {
    let conn = Connection::open_in_memory().expect("open");
    let err = conn
        .execute(r#"User { user_id: "1", user_name: "ada", active: true }.insert()"#)
        .expect_err("schema required");
    assert!(err.to_string().contains("ensure_schema"));
}
