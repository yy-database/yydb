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
    assert!(prepare_sql("SELECT * FROM Account JOIN Other ON Account.account_id = Other.id").is_err());
    assert!(prepare_sql("SELECT * FROM Account GROUP BY tenant").is_err());
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

// ─── LIKE / CASE are rejected by design ─────────────────────────────────

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
    // WHERE account_id = account_id — right-hand side is treated as a
    // parameter placeholder candidate by `lower_value`, so it fails before
    // reaching execution. This confirms column-to-column comparison is not
    // part of the lowering surface.
    let err = prepare_sql("SELECT * FROM Account WHERE account_id = account_id")
        .expect_err("column-to-column comparison must be rejected");
    let msg = format!("{err}");
    assert!(
        msg.contains("rewritten parameter Name") || msg.contains("expected rewritten")
            || msg.contains("unsupported SQL value"),
        "unexpected error: {msg}"
    );
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
fn update_rejects_self_reference_arithmetic() {
    // SET score = score + 1 — RHS is Binary, must be rejected by lowering
    let err = prepare_sql("UPDATE Account SET score = score + 1 WHERE account_id = 1")
        .expect_err("self-ref arithmetic must reject");
    assert!(
        format!("{err}").contains("unsupported") || format!("{err}").contains("arithmetic")
            || format!("{err}").contains("plain column"),
        "unexpected: {err}"
    );
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
