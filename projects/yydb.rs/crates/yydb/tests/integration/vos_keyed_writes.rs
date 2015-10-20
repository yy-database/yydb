//! Phase 1 VOS keyed writes: `Type::update({ … })` and `Type::delete({ … })`.
//!
//! These stay Rust-side because they assert executor contracts the host
//! surface only observes indirectly: that a patch leaves unnamed fields alone,
//! that a failed patch leaves the stored row byte-identical, that a keyed
//! delete is idempotent, and that both survive rollback and reopen.

use std::collections::BTreeMap;

use yydb::{Error, Value, query::QueryRow};

use crate::fixtures::yydb::{cleanup, open_temp_db, reopen};

const SCHEMA: &str = r#"
table Doc {
    @@doc_id: i64,
    @slug: utf8,
    title: utf8,
}
"#;

const MACRO_SCHEMA: &str = r#"
table Doc {
    @@doc_id: i64,
    title: utf8,
}

macro seed_docs() -> unit {
    Doc::insert({ doc_id: 1, title: "one" })
    Doc::insert({ doc_id: 2, title: "two" })
}

macro patch_doc() -> unit {
    Doc::update({ doc_id: 2, title: "macro patched" })
}
"#;

fn insert(doc_id: i64, slug: &str, title: &str) -> String {
    format!(r#"Doc::insert({{ doc_id: {doc_id}, slug: "{slug}", title: "{title}" }})"#)
}

fn update(doc_id: i64, fields: &str) -> String {
    format!(r#"Doc::update({{ doc_id: {doc_id}, {fields} }})"#)
}

fn delete(doc_id: i64) -> String {
    format!(r#"Doc::delete({{ doc_id: {doc_id} }})"#)
}

fn text(row: &QueryRow, field: &str) -> String {
    match row.get(field) {
        Some(Value::Text(value)) => value.clone(),
        other => panic!("`{field}` must be text, got {other:?}"),
    }
}

fn id_of(row: &QueryRow) -> i64 {
    match row.get("doc_id") {
        Some(Value::I64(value)) => value.clone(),
        other => panic!("`doc_id` must be i64, got {other:?}"),
    }
}

#[test]
fn keyed_update_patches_only_the_named_fields() {
    let (conn, path) = open_temp_db("vos-keyed-update");
    conn.ensure_schema(SCHEMA).unwrap();
    conn.execute_vos_write(&insert(1, "alpha", "Alpha")).unwrap();
    conn.execute_vos_write(&insert(2, "beta", "Beta")).unwrap();

    conn.execute_vos_write(&update(1, r#"title: "Alpha patched""#)).unwrap();

    let rows = conn.query_vos("Doc.all().collect()").unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(id_of(&rows[0]), 1);
    assert_eq!(text(&rows[0], "title"), "Alpha patched", "the named field must change");
    assert_eq!(text(&rows[0], "slug"), "alpha", "unnamed fields must keep their stored value");
    assert_eq!(text(&rows[1], "title"), "Beta", "other rows must stay untouched");
    cleanup(&path);
}

#[test]
fn keyed_delete_removes_one_row_and_a_missing_key_is_a_no_op() {
    let (conn, path) = open_temp_db("vos-keyed-delete");
    conn.ensure_schema(SCHEMA).unwrap();
    conn.execute_vos_write(&insert(1, "alpha", "Alpha")).unwrap();
    conn.execute_vos_write(&insert(2, "beta", "Beta")).unwrap();

    conn.execute_vos_write(&delete(1)).unwrap();
    let rows = conn.query_vos("Doc.all().collect()").unwrap();
    assert_eq!(rows.len(), 1, "exactly one row must be removed");
    assert_eq!(id_of(&rows[0]), 2);

    conn.execute_vos_write(&delete(1)).unwrap_or_else(|error| panic!("deleting a missing key must be a no-op: {error}"));
    let rows = conn.query_vos("Doc.all().collect()").unwrap();
    assert_eq!(rows.len(), 1, "a repeated delete must not remove anything else");
    cleanup(&path);
}

#[test]
fn keyed_writes_survive_reopen() {
    let (conn, path) = open_temp_db("vos-keyed-reopen");
    conn.ensure_schema(SCHEMA).unwrap();
    conn.execute_vos_write(&insert(1, "alpha", "Alpha")).unwrap();
    conn.execute_vos_write(&insert(2, "beta", "Beta")).unwrap();
    conn.execute_vos_write(&update(1, r#"title: "Alpha patched""#)).unwrap();
    conn.execute_vos_write(&delete(2)).unwrap();
    conn.close();
    drop(conn);

    let reopened = reopen(&path);
    let rows = reopened.query_vos("Doc.all().collect()").unwrap();
    assert_eq!(rows.len(), 1, "the delete must be durable");
    assert_eq!(id_of(&rows[0]), 1);
    assert_eq!(text(&rows[0], "title"), "Alpha patched", "the patch must be durable");
    assert_eq!(text(&rows[0], "slug"), "alpha");
    reopened.close();
    cleanup(&path);
}

#[test]
fn failed_keyed_update_leaves_the_row_unchanged() {
    let (conn, path) = open_temp_db("vos-keyed-update-conflict");
    conn.ensure_schema(SCHEMA).unwrap();
    conn.execute_vos_write(&insert(1, "alpha", "Alpha")).unwrap();
    conn.execute_vos_write(&insert(2, "beta", "Beta")).unwrap();

    let error = conn.execute_vos_write(&update(2, r#"slug: "alpha""#)).expect_err("a unique conflict must fail the patch");
    assert!(matches!(error, Error::UniqueViolation { .. }), "unexpected error: {error}");

    let rows = conn.query_vos("Doc.all().collect()").unwrap();
    assert_eq!(text(&rows[1], "slug"), "beta", "a rejected patch must not write half a row");
    assert_eq!(text(&rows[1], "title"), "Beta");

    conn.close();
    drop(conn);
    let reopened = reopen(&path);
    let rows = reopened.query_vos("Doc.all().collect()").unwrap();
    assert_eq!(text(&rows[1], "slug"), "beta", "a rejected patch must not reach the file either");
    reopened.close();
    cleanup(&path);
}

#[test]
fn keyed_update_rolls_back_with_its_transaction() {
    let (conn, path) = open_temp_db("vos-keyed-update-rollback");
    conn.ensure_schema(SCHEMA).unwrap();
    conn.execute_vos_write(&insert(1, "alpha", "Alpha")).unwrap();

    conn.begin().unwrap();
    conn.execute_vos_write(&update(1, r#"title: "Never committed""#)).unwrap();
    assert_eq!(text(&conn.query_vos("Doc.all().collect()").unwrap()[0], "title"), "Never committed");
    conn.rollback().unwrap();

    assert_eq!(
        text(&conn.query_vos("Doc.all().collect()").unwrap()[0], "title"),
        "Alpha",
        "rollback must restore the committed row"
    );
    conn.close();
    drop(conn);
    let reopened = reopen(&path);
    assert_eq!(text(&reopened.query_vos("Doc.all().collect()").unwrap()[0], "title"), "Alpha");
    reopened.close();
    cleanup(&path);
}

#[test]
fn keyed_update_requires_the_primary_key() {
    let (conn, path) = open_temp_db("vos-keyed-update-no-key");
    conn.ensure_schema(SCHEMA).unwrap();
    let error = conn
        .execute_vos_write(r#"Doc::update({ title: "Alpha patched" })"#)
        .expect_err("a patch without the primary key cannot select a row");
    assert!(error.to_string().contains("`doc_id`"), "unexpected error: {error}");
    cleanup(&path);
}

#[test]
fn collection_update_stays_unsupported() {
    let (conn, path) = open_temp_db("vos-keyed-collection");
    conn.ensure_schema(SCHEMA).unwrap();
    let error = conn
        .execute_vos_write(r#"Doc.all().update({ title: "Alpha patched" })"#)
        .expect_err("Phase 1 has no predicate evaluator for collection writes");
    assert!(matches!(error, Error::Unsupported(_)), "unexpected error: {error}");
    assert!(error.to_string().contains("table type"), "unexpected error: {error}");
    cleanup(&path);
}

#[test]
fn a_macro_write_program_binds_parameters_and_stores_rows() {
    let (conn, path) = open_temp_db("vos-macro-insert");
    conn.ensure_schema(MACRO_SCHEMA).unwrap();
    let named = BTreeMap::from([("doc_id".to_string(), Value::I64(3)), ("title".to_string(), Value::Text("three".into()))]);
    conn.execute_vos_write_with_params("seed_docs()\nDoc::insert({ doc_id: $doc_id, title: $title })", &named)
        .expect("a macro program that expands to writes must bind its parameters");

    let rows = conn.query_vos("Doc.all().collect()").unwrap();
    assert_eq!(rows.len(), 3, "both macro rows and the parameter row must land");
    assert_eq!(id_of(&rows[2]), 3);
    assert_eq!(text(&rows[2], "title"), "three");

    conn.close();
    drop(conn);
    let reopened = reopen(&path);
    assert_eq!(reopened.query_vos("Doc.all().collect()").unwrap().len(), 3, "the macro rows must be durable");
    reopened.close();
    cleanup(&path);
}

#[test]
fn a_macro_update_program_binds_parameters_and_patches_a_row() {
    let (conn, path) = open_temp_db("vos-macro-update");
    conn.ensure_schema(MACRO_SCHEMA).unwrap();
    conn.execute_vos_write("seed_docs()").unwrap();
    let named = BTreeMap::from([("title".to_string(), Value::Text("parameter patched".into()))]);
    conn.execute_vos_write_with_params("patch_doc()\nDoc::update({ doc_id: 1, title: $title })", &named)
        .expect("a macro program that expands to an update must bind its parameters");

    let rows = conn.query_vos("Doc.all().collect()").unwrap();
    assert_eq!(text(&rows[0], "title"), "parameter patched", "the parameter patch must land");
    assert_eq!(text(&rows[1], "title"), "macro patched", "the macro patch must land too");
    cleanup(&path);
}

#[test]
fn an_unexpandable_macro_write_still_fails() {
    let (conn, path) = open_temp_db("vos-macro-missing");
    conn.ensure_schema(MACRO_SCHEMA).unwrap();
    let error =
        conn.execute_vos_write_with_params("missing_macro()", &BTreeMap::new()).expect_err("an unknown macro must fail");
    assert_eq!(error.code(), "yydb.schema.invalid", "unexpected error: {error}");

    let named = BTreeMap::from([("title".to_string(), Value::Text("ignored".into()))]);
    let error =
        conn.execute_vos_write_with_params("seed_docs($title)", &named).expect_err("a macro call with arguments must fail");
    assert_eq!(error.code(), "yydb.unsupported", "unexpected error: {error}");
    cleanup(&path);
}
