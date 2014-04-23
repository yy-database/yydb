use std::collections::BTreeMap;

use yydb_query::execute_write;

const BLOG_SCHEMA: &str = r#"
table User {
    @@user_id: uuid,
    user_name: utf8,
    active: bool,
}
"#;

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
fn expand_schema_macro_when_vos_supports_document_macros() {
    let Ok(document) = vos::parser::parse_document(BLOG_WITH_MACRO) else {
        return;
    };
    let catalog = vos::catalog_from_document(&document).expect("catalog");
    let mut records = BTreeMap::new();
    execute_write("seed_blog()", &catalog, Some(BLOG_WITH_MACRO), &mut records).expect("execute");
    assert_eq!(records.len(), 2);
}

#[test]
fn static_insert_call_shape() {
    let document = vos::parser::parse_document(BLOG_SCHEMA).expect("schema");
    let catalog = vos::catalog_from_document(&document).expect("catalog");
    let mut records = BTreeMap::new();
    execute_write(
        r#"User::insert({
                user_id: "550e8400-e29b-41d4-a716-446655440000",
                user_name: "ada",
                active: true,
            })"#,
        &catalog,
        None,
        &mut records,
    )
    .expect("execute");
    assert_eq!(records.len(), 1);
}
