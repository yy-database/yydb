//! Catalog metadata for exact scalars and unique constraints.

use std::collections::BTreeMap;

use crate::fixtures::yydb::{cleanup, temp_db};

use yydb::{
    Connection, Error, UniqueConstraint, Value,
    catalog::{FieldPath, RenameMap},
    execution::Type,
};

const EVENT_SCHEMA: &str = r#"
table Event {
    @@event_id: i64,
    @happened_on: date,
    at_time: time,
    created_at: datetime,
    amount: Decimal<38, 2>,
    flat: decimal,
}
"#;

const ACCOUNT_SCHEMA: &str = r#"
table Account {
    @@account_id: i64,
    @email: utf8,
    tenant: utf8,
    handle: utf8,
}
"#;

fn account_insert(account_id: i64, email: &str, tenant: &str, handle: &str) -> String {
    format!(
        r#"Account::insert({{
            account_id: {account_id},
            email: "{email}",
            tenant: "{tenant}",
            handle: "{handle}",
        }})"#
    )
}

fn membership_insert(account_id: i64, email: &str, tenant: &str, alias: &str) -> String {
    format!(
        r#"Membership::insert({{
            account_id: {account_id},
            email: "{email}",
            tenant: "{tenant}",
            alias: "{alias}",
        }})"#
    )
}

#[test]
fn catalog_maps_exact_scalars_and_document_unique_rules() {
    let conn = Connection::open_in_memory().unwrap();
    conn.ensure_schema(EVENT_SCHEMA).unwrap();
    let catalog = conn.execution_catalog().unwrap().expect("catalog");
    let table = catalog.types.iter().find(|entry| entry.name == "Event").expect("Event table");
    let ty = |name: &str| table.fields.iter().find(|field| field.name == name).expect("field").handle.ty();
    assert_eq!(ty("happened_on"), Type::Date);
    assert_eq!(ty("at_time"), Type::Time);
    assert_eq!(ty("created_at"), Type::Timestamp);
    assert_eq!(ty("amount"), Type::Decimal { precision: 38, scale: 2 });
    assert_eq!(ty("flat"), Type::Decimal { precision: 38, scale: 0 });
    assert_eq!(table.unique_constraints.len(), 1);
    assert_eq!(table.unique_constraints[0].name(), "Event_happened_on_key");
    assert_eq!(table.unique_constraints[0].columns(), ["happened_on".to_owned()]);
    assert!(table.unique_constraints[0].nulls_distinct());
}

#[test]
fn declared_unique_constraint_persists_and_blocks_conflicting_writes_after_reopen() {
    let path = temp_db("declared-unique");
    {
        let conn = Connection::open(&path).unwrap();
        conn.ensure_schema(ACCOUNT_SCHEMA).unwrap();
        let constraint =
            UniqueConstraint::new("Account_tenant_handle_key", vec!["tenant".into(), "handle".into()], true).unwrap();
        conn.declare_unique_constraint("Account", constraint).unwrap();
        assert_eq!(conn.unique_constraints().unwrap().len(), 1);
        conn.execute_vos_write(&account_insert(1, "ada@example.test", "t1", "ada")).unwrap();
    }

    let reopened = Connection::open(&path).unwrap();
    let constraints = reopened.unique_constraints().unwrap();
    assert_eq!(constraints.len(), 1);
    assert_eq!(constraints[0].table(), "Account");
    assert_eq!(constraints[0].constraint().columns(), ["tenant".to_owned(), "handle".to_owned()]);
    let catalog = reopened.execution_catalog().unwrap().expect("catalog");
    assert!(catalog.has_unique_constraint("Account", "Account_tenant_handle_key"));

    let error = reopened
        .execute_vos_write(&account_insert(2, "linus@example.test", "t1", "ada"))
        .expect_err("duplicate tenant and handle must conflict");
    assert!(matches!(error, Error::UniqueViolation { .. }), "unexpected error: {error}");
    assert_eq!(error.code(), "yydb.unique.violation");
    assert_eq!(reopened.query_vos("Account.all().collect()").unwrap().len(), 1);
    drop(reopened);
    cleanup(&path);
}

#[test]
fn declared_unique_constraint_rejects_unknown_columns_and_reserves_document_names() {
    let conn = Connection::open_in_memory().unwrap();
    conn.ensure_schema(ACCOUNT_SCHEMA).unwrap();
    let unknown_column = UniqueConstraint::new("Account_missing_key", vec!["missing".into()], true).unwrap();
    assert!(matches!(conn.declare_unique_constraint("Account", unknown_column), Err(Error::Schema { .. })));
    let unknown_table = UniqueConstraint::new("Ghost_tenant_key", vec!["tenant".into()], true).unwrap();
    assert!(matches!(conn.declare_unique_constraint("Ghost", unknown_table), Err(Error::Schema { .. })));
    let reserved = UniqueConstraint::new("Account_email_key", vec!["email".into()], true).unwrap();
    assert!(matches!(conn.declare_unique_constraint("Account", reserved), Err(Error::Schema { .. })));
    assert!(conn.unique_constraints().unwrap().is_empty());
}

#[test]
fn databases_without_declared_rules_reopen_with_empty_metadata() {
    let path = temp_db("legacy-unique");
    {
        let conn = Connection::open(&path).unwrap();
        conn.ensure_schema(ACCOUNT_SCHEMA).unwrap();
        conn.execute_vos_write(&account_insert(1, "ada@example.test", "t1", "ada")).unwrap();
    }
    let reopened = Connection::open(&path).unwrap();
    assert!(reopened.unique_constraints().unwrap().is_empty());
    let rows = reopened.query_vos("Account.all().collect()").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("email"), Some(&Value::Text("ada@example.test".into())));
    drop(reopened);
    cleanup(&path);
}

#[test]
fn migration_remaps_declared_constraints_onto_renamed_tables_and_columns() {
    let conn = Connection::open_in_memory().unwrap();
    conn.ensure_schema(ACCOUNT_SCHEMA).unwrap();
    let constraint = UniqueConstraint::new("Account_tenant_handle_key", vec!["tenant".into(), "handle".into()], true).unwrap();
    conn.declare_unique_constraint("Account", constraint).unwrap();

    let renames = RenameMap {
        types: BTreeMap::from([("Account".into(), "Membership".into())]),
        fields: BTreeMap::from([(FieldPath { type_name: "Account".into(), field_name: "handle".into() }, "alias".into())]),
    };
    conn.migrate_schema("table Membership { @@account_id: i64, email: utf8, tenant: utf8, alias: utf8 }", &renames).unwrap();

    let constraints = conn.unique_constraints().unwrap();
    assert_eq!(constraints.len(), 1);
    assert_eq!(constraints[0].table(), "Membership");
    assert_eq!(constraints[0].constraint().name(), "Account_tenant_handle_key");
    assert_eq!(constraints[0].constraint().columns(), ["tenant".to_owned(), "alias".to_owned()]);
    let catalog = conn.execution_catalog().unwrap().expect("catalog");
    assert!(catalog.has_unique_constraint("Membership", "Account_tenant_handle_key"));

    conn.execute_vos_write(&membership_insert(1, "ada@example.test", "t1", "ada")).unwrap();
    let error = conn
        .execute_vos_write(&membership_insert(2, "linus@example.test", "t1", "ada"))
        .expect_err("remapped composite rule still applies");
    assert!(matches!(error, Error::UniqueViolation { .. }), "unexpected error: {error}");
}
