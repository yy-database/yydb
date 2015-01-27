//! SQL frontend: lowering into shared operators and end-to-end DML execution.

use std::collections::BTreeMap;

use vos::types::ResolvedContract;
use yydb_query::{Dialect, QueryKind, execute_sql, prepare_sql, resolved_contract_from_document};
use yydb_types::Value;

const SCHEMA: &str = r#"
table Account {
    @@account_id: i64,
    email: utf8,
    tenant: utf8,
    score: i64,
    active: bool,
}
"#;

fn contract() -> ResolvedContract {
    resolved_contract_from_document(SCHEMA).expect("account schema resolves")
}

fn exec(sql: &str, values: &[Value], contract: &ResolvedContract, records: &mut BTreeMap<String, Vec<u8>>) {
    execute_sql(sql, values, contract, &[], records).expect("sql executes");
}

fn seed(contract: &ResolvedContract, records: &mut BTreeMap<String, Vec<u8>>) {
    exec(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (1, 'ada@example.test', 't1', 10, true)",
        &[],
        contract,
        records,
    );
    exec(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (2, 'linus@example.test', 't1', 30, false)",
        &[],
        contract,
        records,
    );
    exec(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (3, 'grace@example.test', 't2', 20, true)",
        &[],
        contract,
        records,
    );
}

fn ids(rows: &[BTreeMap<String, Value>]) -> Vec<i64> {
    rows.iter().map(|row| match row.get("account_id") {
        Some(Value::I64(id)) => *id,
        other => panic!("unexpected account_id {other:?}"),
    }).collect()
}

#[test]
fn select_lowers_to_the_shared_read_pipeline() {
    let stmt = prepare_sql("SELECT email FROM Account WHERE score > 10 ORDER BY score DESC LIMIT 2 OFFSET 1").expect("prepare");
    assert_eq!(stmt.dialect, Dialect::Sql);
    assert_eq!(stmt.query_kind, QueryKind::Read);
    assert_eq!(stmt.read_set, vec!["Account".to_owned()]);
    assert!(stmt.parameters.is_empty());
    // Scan -> Filter -> Sort -> Skip -> Take -> Project -> Collect
    assert_eq!(stmt.ops.len(), 7);
}

#[test]
fn select_reads_rows_with_filter_and_ordering() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id, email FROM Account WHERE score >= 20 ORDER BY score DESC",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("select");

    assert_eq!(ids(&outcome.rows), vec![2, 3]);
    assert_eq!(outcome.rows[0].get("email"), Some(&Value::Text("linus@example.test".into())));
}

#[test]
fn select_star_returns_every_stored_column() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql("SELECT * FROM Account WHERE account_id = 1", &[], &contract, &[], &mut records)
        .expect("select star");
    assert_eq!(outcome.rows.len(), 1);
    assert_eq!(outcome.rows[0].get("tenant"), Some(&Value::Text("t1".into())));
}

#[test]
fn positional_parameters_bind_in_declaration_order() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let stmt = prepare_sql("SELECT account_id FROM Account WHERE tenant = ? AND score > ?").expect("prepare");
    assert_eq!(stmt.parameters.len(), 2);
    assert_eq!(stmt.parameters[0].name, "?1");
    assert_eq!(stmt.parameters[1].name, "?2");

    let outcome = execute_sql(
        "SELECT account_id FROM Account WHERE tenant = ? AND score > ?",
        &[Value::Text("t1".into()), Value::I64(20)],
        &contract,
        &[],
        &mut records,
    )
    .expect("parameterized select");
    assert_eq!(ids(&outcome.rows), vec![2]);
}

#[test]
fn insert_update_delete_round_trip() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);
    assert_eq!(records.len(), 3);

    let update = execute_sql("UPDATE Account SET score = 99, email = 'ada@new.test' WHERE account_id = 1", &[], &contract, &[], &mut records)
        .expect("update");
    assert_eq!(update.affected, 1);

    let rows = execute_sql("SELECT score, email FROM Account WHERE account_id = 1", &[], &contract, &[], &mut records)
        .expect("read updated row")
        .rows;
    assert_eq!(rows[0].get("score"), Some(&Value::I64(99)));
    assert_eq!(rows[0].get("email"), Some(&Value::Text("ada@new.test".into())));

    let delete = execute_sql("DELETE FROM Account WHERE active = false", &[], &contract, &[], &mut records).expect("delete");
    assert_eq!(delete.affected, 1);

    let remaining = execute_sql("SELECT account_id FROM Account", &[], &contract, &[], &mut records).expect("read").rows;
    assert_eq!(ids(&remaining), vec![1, 3]);
}

#[test]
fn delete_without_where_clears_the_table() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let delete = execute_sql("DELETE FROM Account", &[], &contract, &[], &mut records).expect("delete all");
    assert_eq!(delete.affected, 3);
    assert!(records.is_empty());
}

#[test]
fn update_rejects_primary_key_reassignment() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let error = execute_sql("UPDATE Account SET account_id = 9 WHERE account_id = 1", &[], &contract, &[], &mut records)
        .expect_err("primary key update must be rejected");
    assert!(format!("{error}").contains("primary key"));
    assert_eq!(records.len(), 3);
}

#[test]
fn ddl_and_joins_are_rejected_explicitly() {
    assert!(prepare_sql("CREATE TABLE T (id INTEGER)").is_err());
    assert!(prepare_sql("SELECT * FROM Account JOIN Other ON Account.account_id = Other.id").is_err());
    assert!(prepare_sql("SELECT * FROM Account GROUP BY tenant").is_err());
}

#[test]
fn sql_statements_mark_their_write_set() {
    let stmt = prepare_sql("DELETE FROM Account WHERE account_id = 1").expect("prepare delete");
    assert_eq!(stmt.query_kind, QueryKind::Write);
    assert_eq!(stmt.write_set, vec!["Account".to_owned()]);
}
