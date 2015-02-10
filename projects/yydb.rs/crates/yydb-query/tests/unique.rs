//! Unique constraint metadata and enforcement on the VOS write path.

use std::collections::BTreeMap;

use vos::types::ResolvedContract;
use yydb_query::{execute, execute_write_with_constraints, resolved_contract_from_document};
use yydb_types::{Error, TableUniqueConstraint, UniqueConstraint, Value};

const ACCOUNT_SCHEMA: &str = r#"
table Account {
    @@account_id: i64,
    @email: utf8,
    tenant: utf8,
    handle: utf8,
}
"#;

fn contract() -> ResolvedContract {
    resolved_contract_from_document(ACCOUNT_SCHEMA).expect("account schema resolves")
}

fn declared(name: &str, columns: &[&str], nulls_distinct: bool) -> TableUniqueConstraint {
    let constraint = UniqueConstraint::new(name, columns.iter().map(|column| (*column).to_owned()).collect(), nulls_distinct)
        .expect("constraint shape");
    TableUniqueConstraint::new("Account", constraint).expect("table binding")
}

fn literal(value: Option<&str>) -> String {
    match value {
        Some(text) => format!("\"{text}\""),
        None => "null".to_owned(),
    }
}

fn insert(
    contract: &ResolvedContract,
    declared: &[TableUniqueConstraint],
    records: &mut BTreeMap<String, Vec<u8>>,
    account_id: i64,
    email: Option<&str>,
    tenant: &str,
    handle: Option<&str>,
) -> yydb_types::Result<()> {
    let source = format!(
        r#"Account::insert({{
            account_id: {account_id},
            email: {},
            tenant: "{tenant}",
            handle: {},
        }})"#,
        literal(email),
        literal(handle),
    );
    execute_write_with_constraints(&source, contract, declared, None, records)
}

fn conflict(error: Error) -> (String, Vec<String>) {
    match error {
        Error::UniqueViolation { constraint, columns, .. } => (constraint, columns),
        other => panic!("expected a unique violation, got {other}"),
    }
}

#[test]
fn unique_attribute_rejects_a_duplicate_single_column_value() {
    let contract = contract();
    let mut records = BTreeMap::new();
    insert(&contract, &[], &mut records, 1, Some("ada@example.test"), "t1", Some("ada")).expect("first insert");
    let (name, columns) = conflict(
        insert(&contract, &[], &mut records, 2, Some("ada@example.test"), "t1", Some("linus")).expect_err("duplicate email"),
    );
    assert_eq!(name, "Account_email_key");
    assert_eq!(columns, vec!["email".to_owned()]);
    assert_eq!(records.len(), 1);
}

#[test]
fn declared_composite_constraint_rejects_a_duplicate_column_set() {
    let contract = contract();
    let composite = declared("Account_tenant_handle_key", &["tenant", "handle"], true);
    let mut records = BTreeMap::new();
    insert(&contract, &[composite.clone()], &mut records, 1, Some("a@example.test"), "t1", Some("ada")).expect("first insert");
    insert(&contract, &[composite.clone()], &mut records, 2, Some("b@example.test"), "t1", Some("linus"))
        .expect("distinct set");
    let (name, columns) = conflict(
        insert(&contract, &[composite], &mut records, 3, Some("c@example.test"), "t1", Some("ada")).expect_err("duplicate set"),
    );
    assert_eq!(name, "Account_tenant_handle_key");
    assert_eq!(columns, vec!["tenant".to_owned(), "handle".to_owned()]);
    assert_eq!(records.len(), 2);
}

#[test]
fn nulls_never_collide_when_nulls_are_distinct() {
    let contract = contract();
    let mut records = BTreeMap::new();
    insert(&contract, &[], &mut records, 1, None, "t1", None).expect("first null row");
    insert(&contract, &[], &mut records, 2, None, "t1", None).expect("second null row");
    assert_eq!(records.len(), 2);
}

#[test]
fn nulls_collide_when_the_constraint_declares_one_shared_null() {
    let contract = contract();
    let single_null = declared("Account_handle_null_key", &["handle"], false);
    let mut records = BTreeMap::new();
    insert(&contract, &[single_null.clone()], &mut records, 1, Some("a@example.test"), "t1", None).expect("first null");
    let (name, columns) = conflict(
        insert(&contract, &[single_null], &mut records, 2, Some("b@example.test"), "t1", None).expect_err("shared null"),
    );
    assert_eq!(name, "Account_handle_null_key");
    assert_eq!(columns, vec!["handle".to_owned()]);
    assert_eq!(records.len(), 1);
}

#[test]
fn conflicting_write_leaves_the_previous_row_intact() {
    let contract = contract();
    let mut records = BTreeMap::new();
    insert(&contract, &[], &mut records, 1, Some("ada@example.test"), "t1", Some("ada")).expect("first insert");
    insert(&contract, &[], &mut records, 2, Some("linus@example.test"), "t1", Some("linus")).expect("second insert");

    insert(&contract, &[], &mut records, 2, Some("ada@example.test"), "t1", Some("linus")).expect_err("conflicting replace");

    let rows = execute("Account.all().collect()", Some(&contract), &records).expect("read rows");
    assert_eq!(rows.len(), 2);
    let second = rows.iter().find(|row| row.get("account_id") == Some(&Value::I64(2))).expect("second row still present");
    assert_eq!(second.get("email"), Some(&Value::Text("linus@example.test".into())));
    assert_eq!(second.get("handle"), Some(&Value::Text("linus".into())));
}

#[test]
fn deleting_a_row_frees_its_unique_key_for_reinsertion() {
    let contract = contract();
    let mut records = BTreeMap::new();
    insert(&contract, &[], &mut records, 1, Some("ada@example.test"), "t1", Some("ada")).expect("first insert");
    records.retain(|key, _| !key.ends_with("/1"));
    assert!(records.is_empty());
    insert(&contract, &[], &mut records, 2, Some("ada@example.test"), "t1", Some("ada")).expect("reinsert after delete");
    assert_eq!(records.len(), 1);
}
