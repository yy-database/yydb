//! Phase 1 VOS text predicates: `.contains()`, `.startsWith()` and `.endsWith()`.
//!
//! These stay Rust-side because they pin the matching unit: a test compares
//! Unicode code points, so it is case sensitive and normalization free. A
//! non-text value must raise a query error instead of silently dropping the
//! row, which the host surface would otherwise report as "no match".

use std::collections::BTreeMap;

use yydb::{Error, Value, query::QueryRow};

use crate::fixtures::yydb::{cleanup, open_temp_db, reopen};

const SCHEMA: &str = r#"
table Doc {
    @@doc_id: i64,
    title: utf8,
    rank: i64,
}
"#;

fn insert(doc_id: i64, title: &str) -> String {
    format!(r#"Doc::insert({{ doc_id: {doc_id}, title: "{title}", rank: {doc_id} }})"#)
}

fn ids(rows: &[QueryRow]) -> Vec<i64> {
    rows.iter()
        .map(|row| match row.get("doc_id") {
            Some(Value::I64(value)) => *value,
            other => panic!("`doc_id` must be i64, got {other:?}"),
        })
        .collect()
}

/// Seed three titles that separate case, composition and word boundaries.
fn seed(conn: &yydb::Connection) {
    conn.ensure_schema(SCHEMA).unwrap();
    conn.execute_vos_write(&insert(1, "Straße")).unwrap();
    conn.execute_vos_write(&insert(2, "café")).unwrap();
    conn.execute_vos_write(&insert(3, "delta")).unwrap();
}

#[test]
fn contains_matches_a_contiguous_code_point_run() {
    let (conn, path) = open_temp_db("vos-text-contains");
    seed(&conn);

    let rows = conn.query_vos(r#"Doc.all().filter(x => x.title.contains("elta")).collect()"#).unwrap();
    assert_eq!(ids(&rows), vec![3], "`elta` must match inside `delta`");

    let rows = conn.query_vos(r#"Doc.all().filter(x => x.title.contains("caf")).collect()"#).unwrap();
    assert_eq!(ids(&rows), vec![2], "`caf` must match the start of `café`");

    let rows = conn.query_vos(r#"Doc.all().filter(x => x.title.contains("afé")).collect()"#).unwrap();
    assert_eq!(ids(&rows), vec![2], "the accented code point must be matched verbatim");

    let rows = conn.query_vos(r#"Doc.all().filter(x => x.title.contains("zeta")).collect()"#).unwrap();
    assert!(rows.is_empty(), "an absent run must match nothing");
    cleanup(&path);
}

#[test]
fn starts_with_and_ends_with_anchor_on_code_point_boundaries() {
    let (conn, path) = open_temp_db("vos-text-anchors");
    seed(&conn);

    let rows = conn.query_vos(r#"Doc.all().filter(x => x.title.startsWith("Str")).collect()"#).unwrap();
    assert_eq!(ids(&rows), vec![1]);

    let rows = conn.query_vos(r#"Doc.all().filter(x => x.title.endsWith("elta")).collect()"#).unwrap();
    assert_eq!(ids(&rows), vec![3]);

    let rows = conn.query_vos(r#"Doc.all().filter(x => x.title.startsWith("elta")).collect()"#).unwrap();
    assert!(rows.is_empty(), "a suffix is not a prefix");

    let rows = conn.query_vos(r#"Doc.all().filter(x => x.title.endsWith("Stra")).collect()"#).unwrap();
    assert!(rows.is_empty(), "a prefix is not a suffix");

    // `é` is one code point, so `caf` ends with the `caf` run but not with the
    // byte sequence that precedes the accent.
    let rows = conn.query_vos(r#"Doc.all().filter(x => x.title.endsWith("afé")).collect()"#).unwrap();
    assert_eq!(ids(&rows), vec![2]);
    cleanup(&path);
}

#[test]
fn matching_is_case_sensitive_and_does_not_normalize() {
    let (conn, path) = open_temp_db("vos-text-exact");
    seed(&conn);

    let rows = conn.query_vos(r#"Doc.all().filter(x => x.title.contains("strasse")).collect()"#).unwrap();
    assert!(rows.is_empty(), "case folding must not turn `Straße` into `strasse`");

    let rows = conn.query_vos(r#"Doc.all().filter(x => x.title.contains("STRASSE")).collect()"#).unwrap();
    assert!(rows.is_empty(), "case folding must not turn `Straße` into `STRASSE`");

    // `é` decomposes into `e` + combining acute. Comparison works on code
    // points, so the decomposed run is a different run.
    let decomposed = "cafe\u{301}";
    let rows = conn.query_vos(&format!(r#"Doc.all().filter(x => x.title.contains("{decomposed}")).collect()"#));
    let rows = rows.unwrap();
    assert!(rows.is_empty(), "NFC and NFD spellings must not be normalized into each other");

    let rows = conn.query_vos(r#"Doc.all().filter(x => x.title.contains("café")).collect()"#).unwrap();
    assert_eq!(ids(&rows), vec![2], "the composed code point must match itself");
    cleanup(&path);
}

#[test]
fn an_empty_needle_matches_every_text_row() {
    let (conn, path) = open_temp_db("vos-text-empty");
    seed(&conn);

    let rows = conn.query_vos(r#"Doc.all().filter(x => x.title.contains("")).collect()"#).unwrap();
    assert_eq!(ids(&rows), vec![1, 2, 3]);
    let rows = conn.query_vos(r#"Doc.all().filter(x => x.title.startsWith("")).collect()"#).unwrap();
    assert_eq!(ids(&rows), vec![1, 2, 3]);
    let rows = conn.query_vos(r#"Doc.all().filter(x => x.title.endsWith("")).collect()"#).unwrap();
    assert_eq!(ids(&rows), vec![1, 2, 3]);
    cleanup(&path);
}

#[test]
fn a_missing_field_never_matches_a_text_test() {
    let (conn, path) = open_temp_db("vos-text-null");
    conn.ensure_schema(SCHEMA).unwrap();
    conn.execute_vos_write("Doc::insert({ doc_id: 1, rank: 1 })").unwrap();

    let rows = conn.query_vos(r#"Doc.all().filter(x => x.title.contains("")).collect()"#).unwrap();
    assert!(rows.is_empty(), "a `NULL` field is unknown, so no text test may keep the row");
    cleanup(&path);
}

#[test]
fn a_text_test_on_a_non_text_field_is_a_query_error() {
    let (conn, path) = open_temp_db("vos-text-non-text");
    seed(&conn);

    let error = conn
        .query_vos(r#"Doc.all().filter(x => x.rank.contains("3")).collect()"#)
        .expect_err("an integer field cannot answer a text test");
    assert!(matches!(error, Error::Schema { .. }), "unexpected error: {error}");
    assert!(error.to_string().contains("`.contains()` requires a text field"), "unexpected error: {error}");
    assert!(error.to_string().contains("`rank`"), "the error must name the field: {error}");
    assert!(error.to_string().contains("`i64`"), "the error must name the stored kind: {error}");
    cleanup(&path);
}

#[test]
fn a_non_text_argument_is_a_query_error() {
    let (conn, path) = open_temp_db("vos-text-argument");
    seed(&conn);

    let mut named = BTreeMap::new();
    named.insert("needle".to_owned(), Value::I64(3));
    let error = conn
        .query_vos_with_params("Doc.all().filter(x => x.title.contains($needle)).collect()", &named)
        .expect_err("an integer argument cannot answer a text test");
    assert!(matches!(error, Error::Schema { .. }), "unexpected error: {error}");
    assert!(error.to_string().contains("`.contains()` requires a text argument"), "unexpected error: {error}");
    assert!(error.to_string().contains("`i64`"), "the error must name the argument kind: {error}");
    cleanup(&path);
}

#[test]
fn text_predicates_read_rows_through_a_reopened_handle() {
    let (conn, path) = open_temp_db("vos-text-reopen");
    seed(&conn);
    conn.close();
    drop(conn);

    let reopened = reopen(&path);
    let rows = reopened.query_vos(r#"Doc.all().filter(x => x.title.endsWith("elta")).collect()"#).unwrap();
    assert_eq!(ids(&rows), vec![3]);
    reopened.close();
    cleanup(&path);
}
