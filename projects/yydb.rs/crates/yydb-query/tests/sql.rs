//! SQL frontend: lowering into shared operators and end-to-end DML execution.

use std::collections::BTreeMap;

use vos::types::ResolvedContract;
use yydb_query::{Dialect, Pred, QueryKind, QueryOp, execute_sql, prepare_sql, resolved_contract_from_document};
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
    rows.iter()
        .map(|row| match row.get("account_id") {
            Some(Value::I64(id)) => *id,
            other => panic!("unexpected account_id {other:?}"),
        })
        .collect()
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

    let outcome =
        execute_sql("SELECT * FROM Account WHERE account_id = 1", &[], &contract, &[], &mut records).expect("select star");
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

    let update = execute_sql(
        "UPDATE Account SET score = 99, email = 'ada@new.test' WHERE account_id = 1",
        &[],
        &contract,
        &[],
        &mut records,
    )
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
    // INNER JOIN is now supported.
    let result = prepare_sql("SELECT * FROM Account JOIN Other ON Account.account_id = Other.id");
    match &result {
        Ok(_) => {}
        Err(e) => eprintln!("JOIN prepare error: {e}"),
    }
    assert!(result.is_ok());
    // GROUP BY is now supported.
    assert!(prepare_sql("SELECT * FROM Account GROUP BY tenant").is_ok());
    // HAVING is still MVP-rejected.
    assert!(prepare_sql("SELECT COUNT(*) FROM Account HAVING COUNT(*) > 1").is_err());
    // Non-INNER joins are still rejected.
    assert!(prepare_sql("SELECT * FROM Account LEFT JOIN Other ON Account.id = Other.id").is_err());
}

#[test]
fn sql_statements_mark_their_write_set() {
    let stmt = prepare_sql("DELETE FROM Account WHERE account_id = 1").expect("prepare delete");
    assert_eq!(stmt.query_kind, QueryKind::Write);
    assert_eq!(stmt.write_set, vec!["Account".to_owned()]);
}

// ─── IS NOT NULL / IS NOT TRUE / IS NOT FALSE ──────────────────────────

#[test]
fn select_with_is_not_null_filters_out_missing_rows() {
    // Insert one row where email is NULL (we cannot easily do this via INSERT VALUES
    // because lowering always requires an explicit column list — but we can check
    // that the lowering itself accepts IS NOT NULL syntax).
    let stmt =
        prepare_sql("SELECT account_id FROM Account WHERE email IS NOT NULL").expect("IS NOT NULL lowers cleanly");
    assert_eq!(stmt.ops.len(), 4); // Scan → Filter → Project → Collect

    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);
    let outcome = execute_sql(
        "SELECT account_id FROM Account WHERE email IS NOT NULL",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("IS NOT NULL selects present rows");
    assert_eq!(ids(&outcome.rows), vec![1, 2, 3]);
}

#[test]
fn select_with_is_not_true_on_bool_column() {
    // `active IS NOT TRUE` → rows where active = false (plus rows where active = NULL)
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id FROM Account WHERE active IS NOT TRUE",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("IS NOT TRUE");
    assert_eq!(ids(&outcome.rows), vec![2]); // only linus has active=false
}

#[test]
fn select_with_is_not_false_on_bool_column() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id FROM Account WHERE active IS NOT FALSE",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("IS NOT FALSE");
    assert_eq!(ids(&outcome.rows), vec![1, 3]); // ada and grace have active=true
}

// ─── IN list: multi-element / single-element ────────────────────────────

#[test]
fn select_with_in_multi_element_list() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id FROM Account WHERE tenant IN ('t1', 't2')",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("IN multi");
    assert_eq!(ids(&outcome.rows), vec![1, 2, 3]);
}

#[test]
fn select_with_in_single_element_list() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id FROM Account WHERE tenant IN ('t2')",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("IN single");
    assert_eq!(ids(&outcome.rows), vec![3]);
}

#[test]
fn select_with_not_in_excludes_matches() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id FROM Account WHERE tenant NOT IN ('t1')",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("NOT IN");
    assert_eq!(ids(&outcome.rows), vec![3]);
}

// ─── BETWEEN / NOT BETWEEN ──────────────────────────────────────────────

#[test]
fn select_with_between_numeric_range() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id FROM Account WHERE score BETWEEN 15 AND 25 ORDER BY score",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("BETWEEN numeric");
    assert_eq!(ids(&outcome.rows), vec![3]); // score=20
}

#[test]
fn select_with_between_inclusive_endpoints() {
    // BETWEEN is inclusive on both ends — 10 AND 30 should include score=10 and score=30
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id FROM Account WHERE score BETWEEN 10 AND 30 ORDER BY score",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("BETWEEN inclusive");
    assert_eq!(ids(&outcome.rows), vec![1, 3, 2]); // 10, 20, 30
}

#[test]
fn select_with_not_between_excludes_range() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id FROM Account WHERE score NOT BETWEEN 10 AND 25",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("NOT BETWEEN");
    assert_eq!(ids(&outcome.rows), vec![2]); // score=30 is outside [10,25]
}

#[test]
fn select_with_between_reversed_endpoints_is_semantically_empty() {
    // `BETWEEN 30 AND 10` — lower endpoint > upper endpoint — the lowering
    // produces (score >= 30 AND score <= 10) which is impossible. Verify
    // the row set is empty rather than a parse/reject error.
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id FROM Account WHERE score BETWEEN 30 AND 10",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("BETWEEN reversed lowers and yields empty");
    assert!(outcome.rows.is_empty());
}

// ─── Compound predicates / nested logic ────────────────────────────────

#[test]
fn select_with_compound_and_of_comparisons() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id FROM Account WHERE tenant = 't1' AND score > 15",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("AND of comparisons");
    assert_eq!(ids(&outcome.rows), vec![2]); // linus in t1 with score 30
}

#[test]
fn select_with_or_of_comparisons() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id FROM Account WHERE account_id = 1 OR account_id = 3 ORDER BY account_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("OR of comparisons");
    assert_eq!(ids(&outcome.rows), vec![1, 3]);
}

#[test]
fn select_with_nested_and_or_logic() {
    // WHERE tenant = 't1' AND (score = 30 OR score = 10)
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id FROM Account WHERE tenant = 't1' AND (score = 30 OR score = 10) ORDER BY score",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("nested AND/OR");
    assert_eq!(ids(&outcome.rows), vec![1, 2]);
}

#[test]
fn select_with_not_grouped_predicate() {
    // WHERE NOT (tenant = 't1')
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id FROM Account WHERE NOT (tenant = 't1')",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("NOT grouped");
    assert_eq!(ids(&outcome.rows), vec![3]); // grace in t2
}

#[test]
fn select_with_not_single_comparison() {
    // WHERE NOT active — identifier alone lowers to FieldBool with expected true
    // but the bare expression is also a valid lowering path (see sql.rs).
    // Let's instead test `WHERE NOT (active = true)` via the negated path.
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id FROM Account WHERE NOT (active IS TRUE)",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("NOT with IS TRUE");
    assert_eq!(ids(&outcome.rows), vec![2]);
}

// ─── LIMIT edge cases ───────────────────────────────────────────────────

#[test]
fn select_with_limit_zero_returns_no_rows() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id FROM Account LIMIT 0",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("LIMIT 0");
    assert!(outcome.rows.is_empty());
}

#[test]
fn select_with_offset_larger_than_result_returns_empty() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    // OFFSET must be combined with LIMIT for oak-sql to parse it (the LIMIT
    // clause wraps both offset and limit). LIMIT 1000 effectively means
    // "take everything left after skip".
    let outcome = execute_sql(
        "SELECT account_id FROM Account ORDER BY account_id LIMIT 1000 OFFSET 99",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("large OFFSET");
    assert!(outcome.rows.is_empty());
}

// ─── ORDER BY multiple columns ─────────────────────────────────────────

#[test]
fn select_with_order_by_multiple_columns() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    // First by tenant (asc), then by score (desc). t1 rows: ada(10), linus(30).
    // After primary sort by tenant asc: t1 rows first, then t2.
    // Within t1, score desc: linus(30), ada(10). t2: grace(20).
    let outcome = execute_sql(
        "SELECT account_id FROM Account ORDER BY tenant ASC, score DESC",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("ORDER BY multi-col");
    assert_eq!(ids(&outcome.rows), vec![2, 1, 3]);
}

// ─── Arithmetic expressions and expression projection ──────────────────
// These are expected to be REJECTED by lowering and the tests confirm that.

#[test]
fn select_rejects_arithmetic_in_projection() {
    let err = prepare_sql("SELECT account_id + 1 FROM Account").expect_err("arithmetic projection must be rejected");
    let msg = format!("{err}");
    assert!(
        msg.contains("unsupported") || msg.contains("plain column"),
        "unexpected error: {msg}"
    );
}

#[test]
fn select_rejects_star_mixed_with_projected_columns() {
    let err = prepare_sql("SELECT *, email FROM Account").expect_err("* cannot mix with other columns");
    assert!(format!("{err}").contains("plain column") || format!("{err}").contains("`*`"));
}

// ─── Value-side arithmetic predicates are rejected ─────────────────────

#[test]
fn select_rejects_arithmetic_on_rhs_of_predicate() {
    // WHERE score = 1 + 1 — value side is arithmetic, must reject
    let err = prepare_sql("SELECT * FROM Account WHERE score = 1 + 1")
        .expect_err("arithmetic value-side predicate must be rejected");
    assert!(
        format!("{err}").contains("unsupported") || format!("{err}").contains("arithmetic"),
        "unexpected: {err}"
    );
}

// ─── LIKE is rejected ──────────────────────────────────────────────────

#[test]
fn select_rejects_like_operator() {
    let err = prepare_sql("SELECT * FROM Account WHERE email LIKE '%@example.test'").expect_err("LIKE must reject");
    assert!(format!("{err}").contains("LIKE"));
}

#[test]
fn select_rejects_without_from_clause() {
    // SELECT 1 — this is a literal value projection without FROM
    let err = prepare_sql("SELECT 1").expect_err("no FROM must reject");
    assert!(format!("{err}").contains("FROM"));
}

// ─── WHERE boolean literal and bare column reference ───────────────────

#[test]
fn select_with_boolean_literal_always_true_predicate() {
    // WHERE true
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id FROM Account WHERE true",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("WHERE true");
    assert_eq!(ids(&outcome.rows), vec![1, 2, 3]);
}

#[test]
fn select_with_boolean_literal_always_false_predicate() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id FROM Account WHERE false",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("WHERE false");
    assert!(outcome.rows.is_empty());
}

#[test]
fn bare_bool_column_reference_as_predicate_is_rejected_by_parser() {
    // WHERE active — oak-sql does not accept a bare column reference as a
    // WHERE predicate. The lowering *would* handle `Expression::Identifier`,
    // but the parser errors first. We confirm this is rejected at parse time
    // so future lowering work knows to update the test if oak-sql ever gains
    // this syntax.
    let err = prepare_sql("SELECT * FROM Account WHERE active")
        .expect_err("bare bool column reference must be rejected at parse time");
    assert!(format!("{err}").contains("predicate") || format!("{err}").contains("parse"));
}

// ─── Column-to-column comparison is intentionally unsupported ──────────

#[test]
fn select_rejects_column_to_column_comparison_on_rhs() {
    // Column-to-column comparison is now supported by lowering (used by
    // JOIN ON and WHERE self-comparison). Verify it lowers to the right Pred.
    let stmt = prepare_sql("SELECT * FROM Account WHERE account_id = account_id")
        .expect("column-to-column comparison now lowers successfully");
    // The Filter predicate should be FieldCmpField.
    let has_field_cmp_field = stmt.ops.iter().any(|op| matches!(
        op, QueryOp::Filter { predicate: Pred::FieldCmpField { .. } }
    ));
    assert!(has_field_cmp_field, "expected FieldCmpField in lowered ops: {stmt:?}");
}

// ─── Multiple statements rejected ──────────────────────────────────────

#[test]
fn multiple_statements_are_rejected() {
    let err = prepare_sql("SELECT * FROM Account; SELECT * FROM Account").expect_err("multi-statement must reject");
    assert!(format!("{err}").contains("one statement"));
}

// ─── UPDATE / DELETE edge cases ────────────────────────────────────────

#[test]
fn update_without_where_affects_every_row() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let update = execute_sql(
        "UPDATE Account SET score = 0",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("update all");
    assert_eq!(update.affected, 3);

    let outcome = execute_sql(
        "SELECT account_id, score FROM Account ORDER BY account_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("read after update all");
    for row in &outcome.rows {
        assert_eq!(row.get("score"), Some(&Value::I64(0)));
    }
}

#[test]
fn update_with_zero_affected_rows_still_succeeds() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let update = execute_sql(
        "UPDATE Account SET score = 99 WHERE account_id = 9999",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("update no-match");
    assert_eq!(update.affected, 0);
    // records unchanged
    assert_eq!(records.len(), 3);
}

#[test]
fn update_self_reference_arithmetic_increments_row() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    // score 10 → 11
    let outcome = execute_sql(
        "UPDATE Account SET score = score + 1 WHERE account_id = 1",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("update arithmetic");
    assert_eq!(outcome.affected, 1);

    let rows = execute_sql(
        "SELECT account_id, score FROM Account ORDER BY account_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("read back");
    assert_eq!(rows.rows.len(), 3);
    assert_eq!(rows.rows[0].get("account_id"), Some(&Value::I64(1)));
    assert_eq!(rows.rows[0].get("score"), Some(&Value::I64(11)));
    // untouched
    assert_eq!(rows.rows[1].get("account_id"), Some(&Value::I64(2)));
    assert_eq!(rows.rows[1].get("score"), Some(&Value::I64(30)));
}

#[test]
fn update_arithmetic_with_parameter_slot() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    // score * score + 2 → 10 * 10 + 2 = 102
    let outcome = execute_sql(
        "UPDATE Account SET score = score * score + 2 WHERE account_id = 1",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("update compound arithmetic");
    assert_eq!(outcome.affected, 1);

    let rows = execute_sql(
        "SELECT account_id, score FROM Account WHERE account_id = 1",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("read back");
    assert_eq!(rows.rows[0].get("score"), Some(&Value::I64(102)));
}

#[test]
fn update_arithmetic_with_bound_parameter() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    // score + ? → 30 + 5 = 35
    let outcome = execute_sql(
        "UPDATE Account SET score = score + ? WHERE account_id = 2",
        &[Value::I64(5)],
        &contract,
        &[],
        &mut records,
    )
    .expect("update arithmetic with bound param");
    assert_eq!(outcome.affected, 1);

    let rows = execute_sql(
        "SELECT score FROM Account WHERE account_id = 2",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("read back");
    assert_eq!(rows.rows[0].get("score"), Some(&Value::I64(35)));
}

// ─── INSERT edge cases ─────────────────────────────────────────────────

#[test]
fn insert_rejects_missing_column_list() {
    // INSERT INTO Account VALUES (...) — no explicit column list
    let err = prepare_sql("INSERT INTO Account VALUES (1, 'x', 't', 0, true)")
        .expect_err("implicit column list must reject");
    assert!(format!("{err}").contains("explicit column list"));
}

#[test]
fn insert_rejects_column_value_count_mismatch() {
    let err = prepare_sql("INSERT INTO Account (account_id, email) VALUES (1)")
        .expect_err("mismatched counts must reject");
    assert!(format!("{err}").contains("count"));
}

#[test]
fn insert_rejects_multiple_value_groups() {
    // lowering only handles a single VALUES clause. The parser may flatten
    // multi-group values which then mismatches the column count, or the
    // parser itself may reject the syntax — either way, an error must occur.
    let err = prepare_sql(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (1, 'a', 't', 0, true), (2, 'b', 't', 1, false)",
    )
    .expect_err("multi-row insert must reject");
    let msg = format!("{err}");
    assert!(
        !msg.is_empty(),
        "expected an error for multi-row insert"
    );
}

// ─── Parameterized DML ──────────────────────────────────────────────────

#[test]
fn insert_bind_positional_parameters_in_values() {
    let contract = contract();
    let mut records = BTreeMap::new();

    let stmt = prepare_sql(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (?, ?, ?, ?, ?)",
    )
    .expect("param insert prepares");
    assert_eq!(stmt.parameters.len(), 5);

    execute_sql(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (?, ?, ?, ?, ?)",
        &[
            Value::I64(4),
            Value::Text("ada@param.test".into()),
            Value::Text("tp".into()),
            Value::I64(99),
            Value::Bool(true),
        ],
        &contract,
        &[],
        &mut records,
    )
    .expect("param insert executes");

    let outcome = execute_sql(
        "SELECT account_id, email, score FROM Account WHERE account_id = 4",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("read param-inserted row");
    assert_eq!(outcome.rows.len(), 1);
    assert_eq!(outcome.rows[0].get("email"), Some(&Value::Text("ada@param.test".into())));
    assert_eq!(outcome.rows[0].get("score"), Some(&Value::I64(99)));
}

#[test]
fn update_with_positional_parameter_in_where() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let update = execute_sql(
        "UPDATE Account SET score = 777 WHERE account_id = ?",
        &[Value::I64(2)],
        &contract,
        &[],
        &mut records,
    )
    .expect("param update");
    assert_eq!(update.affected, 1);

    let outcome = execute_sql(
        "SELECT score FROM Account WHERE account_id = 2",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("read updated");
    assert_eq!(outcome.rows[0].get("score"), Some(&Value::I64(777)));
}

#[test]
fn delete_with_positional_parameter_in_where() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let delete = execute_sql(
        "DELETE FROM Account WHERE tenant = ?",
        &[Value::Text("t1".into())],
        &contract,
        &[],
        &mut records,
    )
    .expect("param delete");
    assert_eq!(delete.affected, 2);

    let remaining = execute_sql(
        "SELECT account_id FROM Account ORDER BY account_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("read remaining");
    assert_eq!(ids(&remaining.rows), vec![3]);
}

// ─── Integer boundary values in INSERT ─────────────────────────────────

#[test]
fn insert_i64_boundary_values() {
    let contract = contract();
    let mut records = BTreeMap::new();

    // I64::MAX
    execute_sql(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (9223372036854775807, 'max@test', 't', 0, true)",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("insert i64 max");

    // A large negative number (close to MIN but safely within range when
    // parsed as UnaryMinus of a Number literal). The exact I64::MIN is not
    // representable as `UnaryMinus(Number(9223372036854775808))` because
    // 9223372036854775808 exceeds i64::MAX — this is a known lowering gap.
    execute_sql(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (-9223372036854775807, 'min@test', 't', 0, false)",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("insert near-min negative");

    // Negative normal
    execute_sql(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (-42, 'neg@test', 't', -100, true)",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("insert negative");

    let outcome = execute_sql(
        "SELECT account_id FROM Account ORDER BY account_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("read boundary ids");
    let got: Vec<i64> = outcome.rows.iter()
        .map(|r| match r.get("account_id") { Some(Value::I64(n)) => *n, _ => panic!() })
        .collect();
    assert_eq!(got, vec![-9223372036854775807, -42, 9223372036854775807]);
}

// ─── Special text values ───────────────────────────────────────────────

#[test]
fn insert_empty_string_and_special_chars() {
    let contract = contract();
    let mut records = BTreeMap::new();

    execute_sql(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (10, '', '', 0, true)",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("insert empty strings");

    // Use parameter binding for strings that contain special characters
    // (apostrophes etc.) because the parser does not perform SQL-style
    // doubled-quote escaping inside literals.
    execute_sql(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (?, ?, ?, 0, true)",
        &[
            Value::I64(11),
            Value::Text("o'brien@test".into()),
            Value::Text("it's".into()),
        ],
        &contract,
        &[],
        &mut records,
    )
    .expect("insert via params");

    let outcome = execute_sql(
        "SELECT account_id, email, tenant FROM Account WHERE account_id >= 10 ORDER BY account_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("read special text");
    assert_eq!(outcome.rows[0].get("email"), Some(&Value::Text("".into())));
    assert_eq!(outcome.rows[1].get("email"), Some(&Value::Text("o'brien@test".into())));
    assert_eq!(outcome.rows[1].get("tenant"), Some(&Value::Text("it's".into())));
}

#[test]
fn insert_unicode_text() {
    let contract = contract();
    let mut records = BTreeMap::new();

    execute_sql(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (12, '邮件@测试.中文', '租户', 0, true)",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("insert unicode");

    let outcome = execute_sql(
        "SELECT email, tenant FROM Account WHERE account_id = 12",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("read unicode");
    assert_eq!(outcome.rows[0].get("email"), Some(&Value::Text("邮件@测试.中文".into())));
    assert_eq!(outcome.rows[0].get("tenant"), Some(&Value::Text("租户".into())));
}

// ─── Boolean edge values in DML ────────────────────────────────────────

#[test]
fn insert_and_filter_with_boolean_values() {
    let contract = contract();
    let mut records = BTreeMap::new();

    execute_sql(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (20, 'a@t', 'x', 0, true)",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("insert true");
    execute_sql(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (21, 'b@t', 'x', 0, false)",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("insert false");

    let active = execute_sql(
        "SELECT account_id FROM Account WHERE active IS TRUE ORDER BY account_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("where true");
    assert_eq!(ids(&active.rows), vec![20]);

    let inactive = execute_sql(
        "SELECT account_id FROM Account WHERE active IS FALSE ORDER BY account_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("where false");
    assert_eq!(ids(&inactive.rows), vec![21]);
}

// ─── UPDATE multiple fields + WHERE combination ─────────────────────────

#[test]
fn update_multiple_fields_then_read_all_back() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    execute_sql(
        "UPDATE Account SET score = 500, email = 'changed@test', tenant = 'zzz' WHERE account_id = 3",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("multi-col update");

    let outcome = execute_sql(
        "SELECT score, email, tenant FROM Account WHERE account_id = 3",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("read updated multi-col");
    assert_eq!(outcome.rows[0].get("score"), Some(&Value::I64(500)));
    assert_eq!(outcome.rows[0].get("email"), Some(&Value::Text("changed@test".into())));
    assert_eq!(outcome.rows[0].get("tenant"), Some(&Value::Text("zzz".into())));
}

// ─── DELETE + INSERT idempotency ──────────────────────────────────────

#[test]
fn delete_then_insert_same_row_preserves_visibility() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    execute_sql(
        "DELETE FROM Account WHERE account_id = 1",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("delete ada");

    execute_sql(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (1, 'ada.2@new.test', 't99', 42, false)",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("reinsert ada");

    let outcome = execute_sql(
        "SELECT email, tenant, score, active FROM Account WHERE account_id = 1",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("read reinserted");
    assert_eq!(outcome.rows[0].get("email"), Some(&Value::Text("ada.2@new.test".into())));
    assert_eq!(outcome.rows[0].get("tenant"), Some(&Value::Text("t99".into())));
    assert_eq!(outcome.rows[0].get("score"), Some(&Value::I64(42)));
    assert_eq!(outcome.rows[0].get("active"), Some(&Value::Bool(false)));
}

// ─── WHERE IS NULL path for non-nullable columns (always empty) ────────

#[test]
fn select_with_is_null_on_populated_column_yields_empty() {
    // All seed rows have non-NULL email, so `email IS NULL` returns empty.
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id FROM Account WHERE email IS NULL",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("IS NULL empty");
    assert!(outcome.rows.is_empty());
}

// ─── UPDATE with NULL value ─────────────────────────────────────────────

#[test]
fn update_can_set_null_on_nullable_column() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let update = execute_sql(
        "UPDATE Account SET tenant = NULL WHERE account_id = 1",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("set null");
    assert_eq!(update.affected, 1);

    let outcome = execute_sql(
        "SELECT tenant FROM Account WHERE account_id = 1",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("read null tenant");
    assert_eq!(outcome.rows[0].get("tenant"), Some(&Value::Null));

    // IS NULL now finds this row
    let nulls = execute_sql(
        "SELECT account_id FROM Account WHERE tenant IS NULL",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("find null tenant");
    assert_eq!(ids(&nulls.rows), vec![1]);
}

// ─── REAL / float literal rejection ─────────────────────────────────────

#[test]
fn select_rejects_real_number_literal_in_predicate() {
    // lowering only supports i64 integer literals. A decimal number like
    // `3.14` should fail in `lower_value` because i64::parse("3.14") fails.
    let err = prepare_sql("SELECT * FROM Account WHERE score = 3.14")
        .expect_err("real literal must be rejected");
    assert!(
        format!("{err}").contains("integer") || format!("{err}").contains("Number")
            || format!("{err}").contains("unsupported"),
        "unexpected: {err}"
    );
}

#[test]
fn insert_rejects_real_number_in_integer_column() {
    let err = prepare_sql(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (1, 'x', 't', 3.14, true)",
    )
    .expect_err("real in integer column must reject");
    assert!(
        format!("{err}").contains("integer") || format!("{err}").contains("Number"),
        "unexpected: {err}"
    );
}

// ─── UUID type column round-trip via parameter binding ──────────────────

const UUID_SCHEMA: &str = r#"
table User {
    @@user_id: uuid,
    user_name: utf8,
    active: bool,
}
"#;

#[test]
fn uuid_column_round_trip_through_sql_parameters() {
    use uuid::Uuid;

    let contract = resolved_contract_from_document(UUID_SCHEMA).expect("uuid schema");
    let mut records = BTreeMap::new();

    let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("parse uuid");

    // UUID must go through parameter binding — no SQL literal syntax for UUID
    execute_sql(
        "INSERT INTO User (user_id, user_name, active) VALUES (?, ?, ?)",
        &[Value::Uuid(id), Value::Text("ada".into()), Value::Bool(true)],
        &contract,
        &[],
        &mut records,
    )
    .expect("uuid insert");

    // Note: the lowering/write path does not yet enforce schema type checks on
    // parameter bindings — the UUID Value gets serialized through the row
    // storage layer which may encode it as text bytes. We verify only that
    // the round-trip is coherent (not that the wire type is preserved).
    let outcome = execute_sql(
        "SELECT user_name FROM User WHERE user_name = 'ada'",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("verify insert landed");
    assert_eq!(outcome.rows.len(), 1);
    assert_eq!(outcome.rows[0].get("user_name"), Some(&Value::Text("ada".into())));
}

#[test]
fn update_uuid_column_via_parameter_binding() {
    use uuid::Uuid;

    let contract = resolved_contract_from_document(UUID_SCHEMA).expect("uuid schema");
    let mut records = BTreeMap::new();
    let id_a = Uuid::parse_str("11111111-1111-1111-1111-111111111111").expect("a");
    let id_b = Uuid::parse_str("22222222-2222-2222-2222-222222222222").expect("b");

    execute_sql(
        "INSERT INTO User (user_id, user_name, active) VALUES (?, ?, true)",
        &[Value::Uuid(id_a), Value::Text("first".into())],
        &contract,
        &[],
        &mut records,
    )
    .expect("insert");

    // Search by the UUID bound as parameter — the lowering path forwards the
    // raw Value into the predicate comparison, which should still match.
    execute_sql(
        "UPDATE User SET user_name = 'renamed' WHERE user_id = ?",
        &[Value::Uuid(id_a)],
        &contract,
        &[],
        &mut records,
    )
    .expect("update by uuid");

    // Verify the UPDATE landed by reading back the row (via a column we know is
    // preserved correctly: user_name).
    let rows = execute_sql(
        "SELECT user_name FROM User WHERE active IS TRUE",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("read after update");
    assert_eq!(rows.rows.len(), 1);
    assert_eq!(rows.rows[0].get("user_name"), Some(&Value::Text("renamed".into())));

    // Updating with a UUID that doesn't match anything should affect zero rows.
    let affected = execute_sql(
        "UPDATE User SET user_name = 'ghost' WHERE user_id = ?",
        &[Value::Uuid(id_b)],
        &contract,
        &[],
        &mut records,
    )
    .expect("update missing");
    assert_eq!(affected.affected, 0);
}

// ─── Large text round-trip ──────────────────────────────────────────────

#[test]
fn large_text_values_round_trip() {
    let contract = contract();
    let mut records = BTreeMap::new();

    let long_email: String = "a".repeat(500);
    let long_tenant: String = "tenant-".repeat(200);

    execute_sql(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (?, ?, ?, 0, true)",
        &[
            Value::I64(9001),
            Value::Text(long_email.clone()),
            Value::Text(long_tenant.clone()),
        ],
        &contract,
        &[],
        &mut records,
    )
    .expect("insert large text");

    let outcome = execute_sql(
        "SELECT email, tenant FROM Account WHERE account_id = 9001",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("read large text");
    assert_eq!(outcome.rows[0].get("email"), Some(&Value::Text(long_email)));
    assert_eq!(outcome.rows[0].get("tenant"), Some(&Value::Text(long_tenant)));
}

// ─── NULL as literal in INSERT ─────────────────────────────────────────

#[test]
fn insert_explicit_null_literal() {
    let contract = contract();
    let mut records = BTreeMap::new();

    // email is nullable per schema (no @@) — INSERT NULL explicitly
    execute_sql(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (9100, NULL, 't', 0, true)",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("insert null literal");

    let outcome = execute_sql(
        "SELECT email FROM Account WHERE account_id = 9100",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("read null");
    assert_eq!(outcome.rows[0].get("email"), Some(&Value::Null));
}

// ─── Aggregate functions: COUNT / SUM / AVG ─────────────────────────────

#[test]
fn aggregate_count_star_global() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql("SELECT COUNT(*) AS c FROM Account", &[], &contract, &[], &mut records).expect("count");
    assert_eq!(outcome.rows.len(), 1);
    assert_eq!(outcome.rows[0].get("c"), Some(&Value::I64(3)));
}

#[test]
fn aggregate_count_expr_skips_null() {
    // First, update one row's tenant to NULL so we can verify COUNT(tenant)
    // excludes it while COUNT(*) counts it.
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);
    exec("UPDATE Account SET tenant = NULL WHERE account_id = 1", &[], &contract, &mut records);

    let outcome =
        execute_sql("SELECT COUNT(*) AS c_all, COUNT(tenant) AS c_nonnull FROM Account", &[], &contract, &[], &mut records)
            .expect("count expr");
    assert_eq!(outcome.rows[0].get("c_all"), Some(&Value::I64(3)));
    assert_eq!(outcome.rows[0].get("c_nonnull"), Some(&Value::I64(2)));
}

#[test]
fn aggregate_sum_global() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql("SELECT SUM(score) AS s FROM Account", &[], &contract, &[], &mut records).expect("sum");
    // 10 + 30 + 20 = 60
    assert_eq!(outcome.rows[0].get("s"), Some(&Value::I64(60)));
}

#[test]
fn aggregate_avg_global() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql("SELECT AVG(score) AS a FROM Account", &[], &contract, &[], &mut records).expect("avg");
    // (10 + 30 + 20) / 3 = 20.0
    assert_eq!(outcome.rows[0].get("a"), Some(&Value::F64(20.0)));
}

#[test]
fn aggregate_avg_with_null_yields_null() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);
    exec("UPDATE Account SET score = NULL WHERE account_id = 1", &[], &contract, &mut records);

    // After the UPDATE, tenant='t1' has linus(30) only — ada's score is NULL.
    let outcome =
        execute_sql("SELECT AVG(score) AS a FROM Account WHERE tenant = 't1'", &[], &contract, &[], &mut records)
            .expect("avg partial");
    assert_eq!(outcome.rows.len(), 1);
    assert_eq!(outcome.rows[0].get("a"), Some(&Value::F64(30.0)));

    // A WHERE clause that matches zero rows should yield NULL for AVG
    let empty =
        execute_sql("SELECT AVG(score) AS a FROM Account WHERE score IS NULL", &[], &contract, &[], &mut records)
            .expect("avg no match");
    assert_eq!(empty.rows.len(), 1);
    assert_eq!(empty.rows[0].get("a"), Some(&Value::Null));
}

#[test]
fn aggregate_count_sum_avg_group_by() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    // t1: ada(10), linus(30) → count=2 sum=40 avg=20
    // t2: grace(20) → count=1 sum=20 avg=20
    let outcome = execute_sql(
        "SELECT tenant, COUNT(*) AS c, SUM(score) AS s, AVG(score) AS a FROM Account GROUP BY tenant ORDER BY tenant",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("grouped aggregates");

    assert_eq!(outcome.rows.len(), 2);
    // t1
    assert_eq!(outcome.rows[0].get("tenant"), Some(&Value::Text("t1".into())));
    assert_eq!(outcome.rows[0].get("c"), Some(&Value::I64(2)));
    assert_eq!(outcome.rows[0].get("s"), Some(&Value::I64(40)));
    assert_eq!(outcome.rows[0].get("a"), Some(&Value::F64(20.0)));
    // t2
    assert_eq!(outcome.rows[1].get("tenant"), Some(&Value::Text("t2".into())));
    assert_eq!(outcome.rows[1].get("c"), Some(&Value::I64(1)));
    assert_eq!(outcome.rows[1].get("s"), Some(&Value::I64(20)));
    assert_eq!(outcome.rows[1].get("a"), Some(&Value::F64(20.0)));
}

// ─── Parameterized value type mismatches ───────────────────────────────
//
// NOTE: the lowering pass currently infers `SlotTy::Any` for every
// positional parameter — it does not yet perform column-aware slot type
// narrowing. A Text value bound to an integer column therefore passes
// lowering silently and the error surfaces (if at all) in the write path
// rather than at bind time. This is documented here so future lowering
// work knows to add type inference and re-enable the negative assertion.
//
// #[test]
// fn parameter_type_mismatch_rejected_at_bind_time() { ... }
