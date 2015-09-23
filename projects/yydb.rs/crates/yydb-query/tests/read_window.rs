//! Read-window derivation: which record-key range one plan may touch.
//!
//! A window is only returned when the plan provably reads from a single table
//! and the primary key is the only thing that bounds it. Every other shape must
//! fall back to a full read.

use vos::types::ResolvedContract;
use yydb_query::{prepare_sql, read_window, resolved_contract_from_document};
use yydb_types::Value;

const SCHEMA: &str = r#"
table Doc {
    @@id: utf8,
    title: utf8,
    rank: i64,
}
"#;

const NUMERIC: &str = r#"
table Point {
    @@id: i64,
    label: utf8,
}
"#;

fn window_with(schema: &str, sql: &str, values: &[Value]) -> Option<yydb_query::ReadWindow> {
    let contract: ResolvedContract = resolved_contract_from_document(schema).expect("schema resolves");
    let stmt = prepare_sql(sql).expect("prepare");
    read_window(&stmt.ops, &contract, values).expect("derive")
}

fn window(sql: &str, values: &[Value]) -> Option<yydb_query::ReadWindow> {
    window_with(SCHEMA, sql, values)
}

#[test]
fn equality_narrows_to_exactly_one_key() {
    let derived = window("SELECT title FROM Doc WHERE id = ?1", &[Value::Text("abc".into())]).expect("window");
    assert_eq!(derived.start, "__yydb/v0/row/Doc/abc");
    assert_eq!(derived.end.as_deref(), Some("__yydb/v0/row/Doc/abc\0"));
}

#[test]
fn half_open_range_maps_to_a_key_range() {
    let derived = window(
        "SELECT title FROM Doc WHERE id >= ?1 AND id < ?2 ORDER BY id",
        &[Value::Text("a".into()), Value::Text("b".into())],
    )
    .expect("window");
    assert_eq!(derived.start, "__yydb/v0/row/Doc/a");
    assert_eq!(derived.end.as_deref(), Some("__yydb/v0/row/Doc/b"));
}

#[test]
fn inclusive_bounds_keep_the_boundary_key() {
    let derived =
        window("SELECT title FROM Doc WHERE id >= ?1 AND id <= ?2", &[Value::Text("a".into()), Value::Text("b".into())])
            .expect("window");
    assert_eq!(derived.start, "__yydb/v0/row/Doc/a");
    assert_eq!(derived.end.as_deref(), Some("__yydb/v0/row/Doc/b\0"));

    let exclusive =
        window("SELECT title FROM Doc WHERE id > ?1 AND id < ?2", &[Value::Text("a".into()), Value::Text("b".into())])
            .expect("window");
    assert_eq!(exclusive.start, "__yydb/v0/row/Doc/a\0");
    assert_eq!(exclusive.end.as_deref(), Some("__yydb/v0/row/Doc/b"));
}

#[test]
fn narrowest_bounds_win() {
    let derived = window(
        "SELECT title FROM Doc WHERE id >= ?1 AND id >= ?2 AND id < ?3",
        &[Value::Text("a".into()), Value::Text("c".into()), Value::Text("m".into())],
    )
    .expect("window");
    assert_eq!(derived.start, "__yydb/v0/row/Doc/c");
    assert_eq!(derived.end.as_deref(), Some("__yydb/v0/row/Doc/m"));
}

#[test]
fn non_primary_predicate_does_not_narrow() {
    assert!(window("SELECT title FROM Doc WHERE title = ?1", &[Value::Text("x".into())]).is_none());
}

#[test]
fn disjunction_does_not_narrow() {
    assert!(
        window("SELECT title FROM Doc WHERE id = ?1 OR id = ?2", &[Value::Text("a".into()), Value::Text("b".into())]).is_none()
    );
}

#[test]
fn unfiltered_scan_does_not_narrow() {
    assert!(window("SELECT title FROM Doc", &[]).is_none());
}

#[test]
fn subquery_predicates_do_not_narrow() {
    assert!(
        window(
            "SELECT title FROM Doc WHERE id = ?1 AND EXISTS (SELECT 1 FROM Doc WHERE id = ?2)",
            &[Value::Text("a".into()), Value::Text("b".into())]
        )
        .is_none()
    );
}

#[test]
fn joins_do_not_narrow() {
    assert!(
        window("SELECT d.title FROM Doc d JOIN Doc other ON d.id = other.id WHERE d.id = ?1", &[Value::Text("a".into())])
            .is_none()
    );
}

/// Only a UTF-8 primary key orders the way the filter compares. A numeric key
/// still narrows on equality, because that names one exact key, but never on a
/// range.
#[test]
fn numeric_primary_key_only_narrows_on_equality() {
    let derived = window_with(NUMERIC, "SELECT label FROM Point WHERE id = ?1", &[Value::I64(7)]).expect("window");
    assert_eq!(derived.start, "__yydb/v0/row/Point/7");
    assert_eq!(derived.end.as_deref(), Some("__yydb/v0/row/Point/7\0"));

    assert!(
        window_with(NUMERIC, "SELECT label FROM Point WHERE id >= ?1 AND id < ?2", &[Value::I64(3), Value::I64(9)]).is_none()
    );
}
