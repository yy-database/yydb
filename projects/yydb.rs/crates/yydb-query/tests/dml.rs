use std::collections::BTreeMap;

use yydb_query::{
    ReadWindow, bind_vos_write, execute_write, execute_write_with_params, point_write_keys, prepare_sql,
    resolved_contract_from_document, write_windows,
};
use yydb_types::{TableUniqueConstraint, UniqueConstraint, Value};

const BLOG_SCHEMA: &str = r#"
table User {
    @@user_id: uuid,
    user_name: utf8,
    active: bool,
}
"#;

const KEYED_SCHEMA: &str = r#"
table Doc {
    @@doc_id: i64,
    title: utf8,
}

table Keyed {
    @@key: utf8,
    @code: utf8,
}

table Tag {
    @@name: utf8,
    label: utf8,
}

table Coded {
    @@name: utf8,
    @code: utf8,
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

const KEYED_WITH_MACRO: &str = r#"
table Doc {
    @@doc_id: i64,
    title: utf8,
}

macro seed_docs() -> unit {
    Doc::insert({ doc_id: 1, title: "one" })
    Doc::insert({ doc_id: 2, title: "two" })
}

macro patch_doc() -> unit {
    Doc::update({ doc_id: 1, title: "one patched" })
}
"#;

#[test]
fn expand_schema_macro_when_vos_supports_document_macros() {
    let contract = resolved_contract_from_document(BLOG_SCHEMA).expect("contract");
    let mut records = BTreeMap::new();
    execute_write("seed_blog()", &contract, Some(BLOG_WITH_MACRO), &mut records).expect("execute");
    assert_eq!(records.len(), 2);
}

#[test]
fn static_insert_call_shape() {
    let contract = resolved_contract_from_document(BLOG_SCHEMA).expect("contract");
    let mut records = BTreeMap::new();
    execute_write(
        r#"User::insert({
                user_id: "550e8400-e29b-41d4-a716-446655440000",
                user_name: "ada",
                active: true,
            })"#,
        &contract,
        None,
        &mut records,
    )
    .expect("execute");
    assert_eq!(records.len(), 1);
}

fn keyed_contract() -> vos::ResolvedContract {
    resolved_contract_from_document(KEYED_SCHEMA).expect("contract")
}

#[test]
fn a_keyed_update_names_exactly_its_primary_key_record() {
    let contract = keyed_contract();
    let keys = point_write_keys(r#"Doc::update({ doc_id: 7, title: "patched" })"#, &contract, &[], Some(KEYED_SCHEMA), &[])
        .expect("plan")
        .expect("a keyed update on a rule-free table must be narrowable");
    assert_eq!(keys, vec!["__yydb/v0/row/Doc/7".to_string()]);
}

#[test]
fn a_keyed_delete_names_exactly_its_primary_key_record() {
    let contract = keyed_contract();
    let keys = point_write_keys(r#"Doc::delete({ doc_id: 7 })"#, &contract, &[], Some(KEYED_SCHEMA), &[])
        .expect("plan")
        .expect("a keyed delete on a rule-free table must be narrowable");
    assert_eq!(keys, vec!["__yydb/v0/row/Doc/7".to_string()], "a delete must read the key so its removal reaches the delta");
}

#[test]
fn an_insert_reads_only_its_target_key() {
    let contract = keyed_contract();
    let keys = point_write_keys(r#"Doc::insert({ doc_id: 7, title: "seven" })"#, &contract, &[], Some(KEYED_SCHEMA), &[])
        .expect("plan")
        .expect("a keyed insert on a rule-free table must be narrowable");
    assert_eq!(keys, vec!["__yydb/v0/row/Doc/7".to_string()]);
}

#[test]
fn a_uniqueness_rule_forces_the_full_path() {
    let contract = keyed_contract();
    let planned =
        point_write_keys(r#"Keyed::insert({ key: "a", code: "A" })"#, &contract, &[], Some(KEYED_SCHEMA), &[]).expect("plan");
    assert!(planned.is_none(), "a unique field needs the whole table, so narrowing must be refused");
}

#[test]
fn a_collection_write_forces_the_full_path() {
    let contract = keyed_contract();
    let planned = point_write_keys(r#"Doc.all().delete()"#, &contract, &[], Some(KEYED_SCHEMA), &[]).expect("plan");
    assert!(planned.is_none(), "a collection write has no single key, so narrowing must be refused");
}

#[test]
fn a_macro_that_expands_to_keyed_writes_stays_narrowable() {
    let contract = resolved_contract_from_document(BLOG_WITH_MACRO).expect("contract");
    let keys = point_write_keys("seed_blog()", &contract, &[], Some(BLOG_WITH_MACRO), &[])
        .expect("plan")
        .expect("a macro of keyed inserts must be narrowable");
    assert_eq!(keys.len(), 2, "both expanded statements must name one key each, got {keys:?}");
}

fn stored_title(contract: &vos::ResolvedContract, records: &BTreeMap<String, Vec<u8>>, id: i64) -> String {
    let rows = yydb_query::execute("Doc.all().collect()", Some(contract), records).expect("read back");
    let row = rows.iter().find(|row| row.get("doc_id") == Some(&Value::I64(id))).expect("row exists");
    match row.get("title") {
        Some(Value::Text(title)) => title.clone(),
        other => panic!("unexpected title {other:?}"),
    }
}

#[test]
fn a_macro_program_binds_parameters_and_stores_inserted_rows() {
    let contract = resolved_contract_from_document(KEYED_WITH_MACRO).expect("contract");
    let source = "seed_docs()\nDoc::insert({ doc_id: $doc_id, title: $title })";
    let named = BTreeMap::from([("doc_id".to_string(), Value::I64(3)), ("title".to_string(), Value::Text("three".into()))]);
    let plan = bind_vos_write(source, &named, Some(KEYED_WITH_MACRO))
        .expect("a macro that expands to inserts must bind like a write program");
    assert_eq!(plan.values, vec![Value::I64(3), Value::Text("three".into())]);

    let mut records = BTreeMap::new();
    execute_write_with_params(&plan.source, &contract, &[], Some(KEYED_WITH_MACRO), &mut records, &plan.values)
        .expect("execute");
    assert_eq!(records.len(), 3, "two macro rows and one parameter row must land");
    assert_eq!(stored_title(&contract, &records, 3), "three");
}

#[test]
fn a_macro_program_binds_parameters_and_patches_an_updated_row() {
    let contract = resolved_contract_from_document(KEYED_WITH_MACRO).expect("contract");
    let source = "patch_doc()\nDoc::update({ doc_id: 1, title: $title })";
    let named = BTreeMap::from([("title".to_string(), Value::Text("patched twice".into()))]);
    let plan = bind_vos_write(source, &named, Some(KEYED_WITH_MACRO))
        .expect("a macro that expands to an update must bind like a write program");

    let mut records = BTreeMap::new();
    execute_write("seed_docs()", &contract, Some(KEYED_WITH_MACRO), &mut records).expect("seed");
    execute_write_with_params(&plan.source, &contract, &[], Some(KEYED_WITH_MACRO), &mut records, &plan.values)
        .expect("execute");
    assert_eq!(stored_title(&contract, &records, 1), "patched twice", "the parameter patch must win over the macro patch");
}

fn bind_error(source: &str, named: &BTreeMap<String, Value>, document: Option<&str>) -> yydb_types::Error {
    bind_vos_write(source, named, document).err().expect("write binding must fail")
}

#[test]
fn a_macro_without_a_schema_document_still_fails() {
    let err = bind_error("seed_docs()", &BTreeMap::new(), None);
    assert_eq!(err.code(), "yydb.schema.invalid", "an unexpandable macro must stay fail-closed");
}

#[test]
fn an_unknown_macro_still_fails() {
    let err = bind_error("missing_macro()", &BTreeMap::new(), Some(KEYED_WITH_MACRO));
    assert_eq!(err.code(), "yydb.schema.invalid", "an unknown macro must not pass as a write");
}

#[test]
fn a_macro_call_with_a_parameter_argument_still_fails() {
    let named = BTreeMap::from([("title".to_string(), Value::Text("ignored".into()))]);
    let err = bind_error("seed_docs($title)", &named, Some(KEYED_WITH_MACRO));
    assert_eq!(err.code(), "yydb.unsupported", "a macro call with arguments must not be accepted");
}

#[test]
fn a_read_pipeline_is_still_refused_by_write_binding() {
    let err = bind_error("Doc.all().collect()", &BTreeMap::new(), Some(KEYED_WITH_MACRO));
    assert_eq!(err.code(), "yydb.unsupported", "a read pipeline must never enter the write executor");
}

/// Plan the record windows of one prepared SQL write statement.
fn sql_windows(source: &str, values: &[Value]) -> Option<Vec<ReadWindow>> {
    let contract = keyed_contract();
    let stmt = prepare_sql(source).expect("prepare");
    let bound = yydb_query::bind_ordered(&stmt, values).expect("bind");
    write_windows(&stmt.write_ops, &contract, &[], &bound).expect("plan")
}

fn doc_key(id: i64) -> String {
    format!("__yydb/v0/row/Doc/{id}")
}

fn single_row_window(id: i64) -> ReadWindow {
    let key = doc_key(id);
    ReadWindow { end: Some(format!("{key}\u{0}")), start: key }
}

#[test]
fn an_sql_insert_names_only_its_target_row() {
    let windows = sql_windows("INSERT INTO Doc (doc_id, title) VALUES (?1, ?2)", &[Value::I64(7), Value::Text("seven".into())])
        .expect("a rule-free single-row insert must be narrowable");
    assert_eq!(windows, vec![single_row_window(7)]);
}

#[test]
fn a_multi_row_sql_insert_names_every_target_row() {
    let windows = sql_windows(
        "INSERT INTO Doc (doc_id, title) VALUES (?1, ?2), (?3, ?4)",
        &[Value::I64(7), Value::Text("seven".into()), Value::I64(8), Value::Text("eight".into())],
    )
    .expect("a rule-free multi-row insert must be narrowable");
    assert_eq!(windows, vec![single_row_window(7), single_row_window(8)]);
}

#[test]
fn a_sql_update_bounded_by_the_primary_key_names_its_row() {
    let windows = sql_windows("UPDATE Doc SET title = ?1 WHERE doc_id = ?2", &[Value::Text("patched".into()), Value::I64(7)])
        .expect("an equality-bounded update must be narrowable");
    assert_eq!(windows, vec![single_row_window(7)]);
}

#[test]
fn a_sql_delete_bounded_by_the_primary_key_names_its_row() {
    let windows = sql_windows("DELETE FROM Doc WHERE doc_id = ?1", &[Value::I64(7)])
        .expect("an equality-bounded delete must be narrowable");
    assert_eq!(windows, vec![single_row_window(7)], "a delete must read the row so its removal reaches the delta");
}

#[test]
fn a_text_primary_key_range_becomes_a_record_range() {
    let windows = sql_windows("DELETE FROM Tag WHERE name >= ?1", &[Value::Text("b".into())])
        .expect("a text primary-key range orders like the record tree, so it must be narrowable");
    assert_eq!(
        windows,
        vec![ReadWindow { start: "__yydb/v0/row/Tag/b".to_string(), end: None }],
        "an open range keeps the upper bound open"
    );
}

#[test]
fn a_numeric_primary_key_range_forces_the_full_path() {
    let planned = sql_windows("DELETE FROM Doc WHERE doc_id >= ?1", &[Value::I64(5)]);
    assert!(planned.is_none(), "numeric key text order differs from numeric order, so no record range exists");
}

#[test]
fn an_unbounded_sql_delete_forces_the_full_path() {
    assert!(
        sql_windows("DELETE FROM Doc", &[]).is_none(),
        "a table-wide delete reaches every row, so narrowing must be refused"
    );
}

#[test]
fn a_non_key_sql_update_forces_the_full_path() {
    let planned =
        sql_windows("UPDATE Doc SET title = ?1 WHERE title = ?2", &[Value::Text("new".into()), Value::Text("old".into())]);
    assert!(planned.is_none(), "a predicate that does not bound the primary key cannot name a window");
}

#[test]
fn an_sql_insert_into_a_document_rule_table_forces_the_full_path() {
    let planned =
        sql_windows("INSERT INTO Coded (name, code) VALUES (?1, ?2)", &[Value::Text("a".into()), Value::Text("A".into())]);
    assert!(planned.is_none(), "a uniqueness rule declared in the schema needs the whole table");
}

#[test]
fn an_sql_insert_into_a_host_rule_table_forces_the_full_path() {
    let contract = keyed_contract();
    let stmt = prepare_sql("INSERT INTO Doc (doc_id, title) VALUES (?1, ?2)").expect("prepare");
    let bound = yydb_query::bind_ordered(&stmt, &[Value::I64(7), Value::Text("seven".into())]).expect("bind");
    let rule =
        TableUniqueConstraint::new("Doc", UniqueConstraint::new("doc_title", vec!["title".into()], false).expect("rule"))
            .expect("binding");
    assert!(
        write_windows(&stmt.write_ops, &contract, &[rule], &bound).expect("plan").is_none(),
        "a host-declared rule needs the whole table exactly like a document rule"
    );
}

#[test]
fn an_on_conflict_clause_forces_the_full_path() {
    let planned = sql_windows(
        "INSERT INTO Doc (doc_id, title) VALUES (?1, ?2) ON CONFLICT (doc_id) DO NOTHING",
        &[Value::I64(7), Value::Text("seven".into())],
    );
    assert!(planned.is_none(), "an ON CONFLICT clause can read the conflicting row, so narrowing must be refused");
}
