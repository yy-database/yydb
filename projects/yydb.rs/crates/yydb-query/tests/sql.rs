//! SQL frontend: lowering into shared operators and end-to-end DML execution.

use std::collections::BTreeMap;

use vos::types::ResolvedContract;
use yydb_query::{Dialect, Pred, QueryKind, QueryOp, WriteOp, execute_sql, prepare_sql, resolved_contract_from_document};
use yydb_types::{Error, TableUniqueConstraint, UniqueConstraint, Value};

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
fn ddl_and_remaining_unsupported_clauses_are_rejected_explicitly() {
    assert!(prepare_sql("CREATE TABLE T (id INTEGER)").is_err());
    // INNER JOIN is supported.
    assert!(prepare_sql("SELECT * FROM Account JOIN Other ON Account.account_id = Other.id").is_ok());
    // LEFT JOIN is also supported now.
    assert!(prepare_sql("SELECT * FROM Account LEFT JOIN Other ON Account.id = Other.id").is_ok());
    // RIGHT / FULL joins lower into the shared join operator now.
    assert!(prepare_sql("SELECT * FROM Account RIGHT JOIN Other ON Account.id = Other.id").is_ok());
    assert!(prepare_sql("SELECT * FROM Account FULL JOIN Other ON Account.id = Other.id").is_ok());
    // Self joins lower when both occurrences are aliased, and stay rejected
    // without aliases because one reference cannot tell them apart.
    assert!(prepare_sql("SELECT * FROM Account AS a JOIN Account AS b ON a.account_id = b.account_id").is_ok());
    let err = prepare_sql("SELECT * FROM Account JOIN Account ON Account.account_id = Account.account_id")
        .expect_err("unaliased self join must reject");
    assert!(format!("{err}").contains("referenced more than once"), "unexpected self join error: {err}");
    // GROUP BY is now supported.
    assert!(prepare_sql("SELECT * FROM Account GROUP BY tenant").is_ok());
    // HAVING is still MVP-rejected.
    assert!(prepare_sql("SELECT COUNT(*) FROM Account HAVING COUNT(*) > 1").is_err());
}

// ─── LEFT JOIN end-to-end ──────────────────────────────────────────────

const LEFT_JOIN_SCHEMA: &str = r#"
table Account {
    @@account_id: i64,
    email: utf8,
    tenant: utf8,
    score: i64,
    active: bool,
}
table Project {
    @@project_id: i64,
    owner_id: i64,
    name: utf8,
}
"#;

fn left_join_contract() -> ResolvedContract {
    resolved_contract_from_document(LEFT_JOIN_SCHEMA).expect("left join schema resolves")
}

fn seed_left_join(contract: &ResolvedContract, records: &mut BTreeMap<String, Vec<u8>>) {
    // Account 1 and 2 have projects, Account 3 has no project at all.
    exec(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (1, 'ada@test', 't1', 10, true)",
        &[],
        contract,
        records,
    );
    exec(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (2, 'linus@test', 't1', 30, false)",
        &[],
        contract,
        records,
    );
    exec(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (3, 'grace@test', 't2', 20, true)",
        &[],
        contract,
        records,
    );
    exec("INSERT INTO Project (project_id, owner_id, name) VALUES (10, 1, 'p1')", &[], contract, records);
    exec("INSERT INTO Project (project_id, owner_id, name) VALUES (11, 1, 'p2')", &[], contract, records);
    exec("INSERT INTO Project (project_id, owner_id, name) VALUES (20, 2, 'p3')", &[], contract, records);
}

#[test]
fn left_join_keeps_all_left_rows_padding_unmatched_with_null() {
    let contract = left_join_contract();
    let mut records = BTreeMap::new();
    seed_left_join(&contract, &mut records);

    // LEFT JOIN Account × Project — expect:
    // account_id=1 matches twice (p1, p2)
    // account_id=2 matches once (p3)
    // account_id=3 has no match → padded row with Project.* = NULL
    let outcome = execute_sql(
        "SELECT account_id, Project.name AS pname FROM Account LEFT JOIN Project ON Account.account_id = Project.owner_id ORDER BY account_id, pname",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("left join");

    assert_eq!(outcome.rows.len(), 4);

    // ada × p1
    assert_eq!(outcome.rows[0].get("account_id"), Some(&Value::I64(1)));
    assert_eq!(outcome.rows[0].get("pname"), Some(&Value::Text("p1".into())));
    // ada × p2
    assert_eq!(outcome.rows[1].get("account_id"), Some(&Value::I64(1)));
    assert_eq!(outcome.rows[1].get("pname"), Some(&Value::Text("p2".into())));
    // linus × p3
    assert_eq!(outcome.rows[2].get("account_id"), Some(&Value::I64(2)));
    assert_eq!(outcome.rows[2].get("pname"), Some(&Value::Text("p3".into())));
    // grace unmatched → pname is NULL
    assert_eq!(outcome.rows[3].get("account_id"), Some(&Value::I64(3)));
    assert_eq!(outcome.rows[3].get("pname"), Some(&Value::Null));
}

#[test]
fn left_join_with_where_filter_on_left_removes_padded_rows() {
    let contract = left_join_contract();
    let mut records = BTreeMap::new();
    seed_left_join(&contract, &mut records);

    // SQL semantics: FROM/JOIN produce a virtual table first, then WHERE
    // filters it. So grace's padded row is generated first (Project.* all
    // NULL) and then WHERE tenant = 't1' drops it.
    let outcome = execute_sql(
        "SELECT account_id, Project.name AS pname FROM Account LEFT JOIN Project ON Account.account_id = Project.owner_id WHERE Account.tenant = 't1' ORDER BY account_id, pname",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("left join with left filter");

    // account 1 has two projects, account 2 has one = 3 rows.
    assert_eq!(outcome.rows.len(), 3);
    for row in &outcome.rows {
        assert_ne!(row.get("account_id"), Some(&Value::I64(3)));
    }
}

#[test]
fn left_join_with_empty_right_table_yields_all_left_with_nulls() {
    let contract = left_join_contract();
    let mut records = BTreeMap::new();
    // Only seed Account, leave Project completely empty.
    exec(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (1, 'a@t', 't', 0, true)",
        &[],
        &contract,
        &mut records,
    );
    exec(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (2, 'b@t', 't', 0, false)",
        &[],
        &contract,
        &mut records,
    );

    let outcome = execute_sql(
        "SELECT account_id, Project.project_id AS pid FROM Account LEFT JOIN Project ON Account.account_id = Project.owner_id ORDER BY account_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("left join empty right");

    assert_eq!(outcome.rows.len(), 2);
    assert_eq!(outcome.rows[0].get("pid"), Some(&Value::Null));
    assert_eq!(outcome.rows[1].get("pid"), Some(&Value::Null));
}

// ─── RIGHT / FULL JOIN end-to-end ──────────────────────────────────────

fn seed_orphan_projects(contract: &ResolvedContract, records: &mut BTreeMap<String, Vec<u8>>) {
    // owner_id 99 / 88 have no Account row at all.
    exec("INSERT INTO Project (project_id, owner_id, name) VALUES (30, 99, 'p4')", &[], contract, records);
    exec("INSERT INTO Project (project_id, owner_id, name) VALUES (40, 88, 'p5')", &[], contract, records);
}

fn seed_account(contract: &ResolvedContract, records: &mut BTreeMap<String, Vec<u8>>, id: i64, active: bool) {
    let sql = format!(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES ({id}, 'seed@test', 't', 0, {active})"
    );
    exec(&sql, &[], contract, records);
}

/// Read `account_id` and the projected `Project.name AS pname` from each row,
/// sorted so assertions do not depend on join emission order. `None` means the
/// column is `NULL` (missing or padded).
fn join_pairs(rows: &[BTreeMap<String, Value>]) -> Vec<(Option<i64>, Option<String>)> {
    let mut pairs: Vec<(Option<i64>, Option<String>)> = rows
        .iter()
        .map(|row| {
            let account_id = match row.get("account_id") {
                Some(Value::I64(id)) => Some(*id),
                Some(Value::Null) | None => None,
                other => panic!("unexpected account_id {other:?}"),
            };
            let name = match row.get("pname") {
                Some(Value::Text(text)) => Some(text.clone()),
                Some(Value::Null) | None => None,
                other => panic!("unexpected pname {other:?}"),
            };
            (account_id, name)
        })
        .collect();
    pairs.sort();
    pairs
}

#[test]
fn right_join_keeps_all_right_rows_padding_unmatched_with_null() {
    let contract = left_join_contract();
    let mut records = BTreeMap::new();
    seed_left_join(&contract, &mut records);
    seed_orphan_projects(&contract, &mut records);

    // RIGHT JOIN Account × Project — every Project row survives:
    // p1/p2 → account 1, p3 → account 2, p4/p5 → no account (NULL left side).
    let outcome = execute_sql(
        "SELECT account_id, Project.name AS pname FROM Account RIGHT JOIN Project ON Account.account_id = Project.owner_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("right join");

    assert_eq!(
        join_pairs(&outcome.rows),
        vec![
            (None, Some("p4".to_owned())),
            (None, Some("p5".to_owned())),
            (Some(1), Some("p1".to_owned())),
            (Some(1), Some("p2".to_owned())),
            (Some(2), Some("p3".to_owned())),
        ]
    );
}

#[test]
fn right_join_with_empty_left_table_pads_every_right_row() {
    let contract = left_join_contract();
    let mut records = BTreeMap::new();
    // Only Project rows; Account stays empty.
    exec("INSERT INTO Project (project_id, owner_id, name) VALUES (10, 1, 'p1')", &[], &contract, &mut records);
    exec("INSERT INTO Project (project_id, owner_id, name) VALUES (20, 2, 'p2')", &[], &contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id, Project.name AS pname FROM Account RIGHT JOIN Project ON Account.account_id = Project.owner_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("right join empty left");

    assert_eq!(join_pairs(&outcome.rows), vec![(None, Some("p1".to_owned())), (None, Some("p2".to_owned()))]);
}

#[test]
fn right_join_with_empty_right_table_yields_no_rows() {
    let contract = left_join_contract();
    let mut records = BTreeMap::new();
    // Only Account rows; Project stays empty.
    seed_account(&contract, &mut records, 1, true);

    let outcome = execute_sql(
        "SELECT account_id, Project.name AS pname FROM Account RIGHT JOIN Project ON Account.account_id = Project.owner_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("right join empty right");

    // A right join keeps only right-hand rows, so an empty right table yields
    // nothing regardless of the left side.
    assert!(outcome.rows.is_empty());
}

#[test]
fn right_join_without_matches_pads_every_right_row() {
    let contract = left_join_contract();
    let mut records = BTreeMap::new();
    // Accounts exist, but no project points at them.
    seed_account(&contract, &mut records, 1, true);
    seed_account(&contract, &mut records, 2, false);
    exec("INSERT INTO Project (project_id, owner_id, name) VALUES (30, 99, 'p4')", &[], &contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id, Project.name AS pname FROM Account RIGHT JOIN Project ON Account.account_id = Project.owner_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("right join without matches");

    assert_eq!(join_pairs(&outcome.rows), vec![(None, Some("p4".to_owned()))]);
}

#[test]
fn full_join_keeps_unmatched_rows_from_both_sides() {
    let contract = left_join_contract();
    let mut records = BTreeMap::new();
    seed_left_join(&contract, &mut records);
    seed_orphan_projects(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id, Project.name AS pname FROM Account FULL JOIN Project ON Account.account_id = Project.owner_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("full join");

    assert_eq!(
        join_pairs(&outcome.rows),
        vec![
            // right rows with no account
            (None, Some("p4".to_owned())),
            (None, Some("p5".to_owned())),
            // matched rows
            (Some(1), Some("p1".to_owned())),
            (Some(1), Some("p2".to_owned())),
            (Some(2), Some("p3".to_owned())),
            // account with no project
            (Some(3), None),
        ]
    );
}

#[test]
fn full_join_with_empty_right_table_pads_every_left_row() {
    let contract = left_join_contract();
    let mut records = BTreeMap::new();
    seed_account(&contract, &mut records, 1, true);
    seed_account(&contract, &mut records, 2, false);

    let outcome = execute_sql(
        "SELECT account_id, Project.name AS pname FROM Account FULL JOIN Project ON Account.account_id = Project.owner_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("full join empty right");

    assert_eq!(join_pairs(&outcome.rows), vec![(Some(1), None), (Some(2), None)]);
}

#[test]
fn full_join_with_empty_tables_yields_no_rows() {
    let contract = left_join_contract();
    let mut records = BTreeMap::new();

    let outcome = execute_sql(
        "SELECT account_id, Project.name AS pname FROM Account FULL JOIN Project ON Account.account_id = Project.owner_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("full join empty tables");

    assert!(outcome.rows.is_empty());
}

#[test]
fn join_on_keeps_outer_rows_that_where_then_removes() {
    // `ON` decides which pairs match, so restricting it in a RIGHT JOIN turns
    // the excluded right rows into NULL-padded rows. `WHERE` runs after the
    // join and therefore filters those padded rows away again.
    let contract = left_join_contract();
    let mut records = BTreeMap::new();
    seed_left_join(&contract, &mut records);
    seed_orphan_projects(&contract, &mut records);

    let on_restricted = execute_sql(
        "SELECT account_id, Project.name AS pname FROM Account RIGHT JOIN Project ON Account.account_id = Project.owner_id AND Project.name = 'p1'",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("right join with restricted on");

    assert_eq!(
        join_pairs(&on_restricted.rows),
        vec![
            (None, Some("p2".to_owned())),
            (None, Some("p3".to_owned())),
            (None, Some("p4".to_owned())),
            (None, Some("p5".to_owned())),
            (Some(1), Some("p1".to_owned())),
        ]
    );

    let where_filtered = execute_sql(
        "SELECT account_id, Project.name AS pname FROM Account RIGHT JOIN Project ON Account.account_id = Project.owner_id WHERE Project.name = 'p1'",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("right join with where");

    assert_eq!(join_pairs(&where_filtered.rows), vec![(Some(1), Some("p1".to_owned()))]);
}

#[test]
fn every_join_kind_emits_the_same_merged_row_shape() {
    let contract = left_join_contract();
    let mut records = BTreeMap::new();
    seed_left_join(&contract, &mut records);

    // Base table columns stay bare, joined table columns stay prefixed, so a
    // padded row from either side has the same columns as a matched row.
    let expected_keys = vec![
        "Project.name",
        "Project.owner_id",
        "Project.project_id",
        "account_id",
        "active",
        "email",
        "score",
        "tenant",
    ];

    for kind in ["JOIN", "LEFT JOIN", "RIGHT JOIN", "FULL JOIN"] {
        let sql = format!("SELECT * FROM Account {kind} Project ON Account.account_id = Project.owner_id");
        let outcome = execute_sql(&sql, &[], &contract, &[], &mut records).expect("join kind executes");
        assert!(!outcome.rows.is_empty(), "{kind} produced no rows");
        for row in &outcome.rows {
            let keys: Vec<&str> = row.keys().map(String::as_str).collect();
            assert_eq!(keys, expected_keys, "{kind} changed the merged row shape");
        }
    }
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
        msg.contains("projection") || msg.contains("arithmetic") || msg.contains("binary") || msg.contains("unsupported"),
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
fn insert_lowers_every_values_row_into_one_insert_op() {
    let stmt = prepare_sql(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (1, 'a', 't', 0, true), (2, 'b', 't', 1, false)",
    )
    .expect("multi-row insert prepares");
    match stmt.write_ops.as_slice() {
        [WriteOp::Insert { rows, .. }] => {
            assert_eq!(rows.len(), 2, "row boundaries come from the VALUES list");
            assert_eq!(rows[0][0].name, "account_id");
            assert_eq!(rows[1][0].name, "account_id");
        }
        other => panic!("expected exactly one insert op, got {other:?}"),
    }
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

// ─── COUNT(DISTINCT col) via SELECT DISTINCT COUNT(col) ────────────────

#[test]
fn select_distinct_count_collapses_duplicate_values() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    // scores = 10, 30, 20 — all distinct → COUNT(DISTINCT score) = 3
    let distinct = execute_sql(
        "SELECT DISTINCT COUNT(score) AS c FROM Account",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("distinct count");
    assert_eq!(distinct.rows[0].get("c"), Some(&Value::I64(3)));

    // Now make two rows have the same score
    exec("UPDATE Account SET score = 10 WHERE account_id = 2", &[], &contract, &mut records);
    // scores = 10, 10, 20 — distinct = {10, 20} → 2
    let distinct2 = execute_sql(
        "SELECT DISTINCT COUNT(score) AS c FROM Account",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("distinct count duplicates");
    assert_eq!(distinct2.rows[0].get("c"), Some(&Value::I64(2)));

    // Plain COUNT(score) still counts all non-null rows
    let plain = execute_sql(
        "SELECT COUNT(score) AS c FROM Account",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("plain count");
    assert_eq!(plain.rows[0].get("c"), Some(&Value::I64(3)));
}

#[test]
fn select_distinct_count_skips_null_values() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);
    // Set score NULL on ada and linus (grace keeps 20)
    exec("UPDATE Account SET score = NULL WHERE account_id IN (1, 2)", &[], &contract, &mut records);

    let distinct = execute_sql(
        "SELECT DISTINCT COUNT(score) AS c FROM Account",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("distinct count nulls");
    // Only grace has score=20 → one distinct non-null value
    assert_eq!(distinct.rows[0].get("c"), Some(&Value::I64(1)));
}

#[test]
fn select_distinct_count_star_is_rejected() {
    // SELECT DISTINCT COUNT(*) is ambiguous at lowering time.
    let err = prepare_sql("SELECT DISTINCT COUNT(*) AS c FROM Account")
        .expect_err("DISTINCT COUNT(*) must reject");
    assert!(format!("{err}").contains("ambiguous"));
}

#[test]
fn select_distinct_does_not_change_plain_sum() {
    // SELECT DISTINCT paired with non-COUNT aggregate is ignored (MVP).
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT DISTINCT SUM(score) AS s FROM Account",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("distinct sum");
    assert_eq!(outcome.rows[0].get("s"), Some(&Value::I64(60))); // 10 + 30 + 20
}

// ─── CASE WHEN ... THEN ... [ELSE ...] END ───────────────────────────────

// These tests require oak-sql to have builder support for
// `Expression::Case`. The lowering path and executor are already in place;
// re-enable once the parser emits CASE AST nodes.

#[test]
fn case_when_projection_first_matching_branch_wins() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id, CASE WHEN score > 20 THEN 'high' WHEN score > 10 THEN 'mid' ELSE 'low' END AS bucket FROM Account ORDER BY account_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("CASE WHEN bucket");
    // ada(10) → 'low', linus(30) → 'high', grace(20) → 'mid'
    assert_eq!(outcome.rows[0].get("bucket"), Some(&Value::Text("low".into())));
    assert_eq!(outcome.rows[1].get("bucket"), Some(&Value::Text("high".into())));
    assert_eq!(outcome.rows[2].get("bucket"), Some(&Value::Text("mid".into())));
}

#[test]
fn case_when_without_else_yields_null() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id, CASE WHEN score > 25 THEN 'top' END AS t FROM Account ORDER BY account_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("CASE WHEN no ELSE");
    // linus(30) → 'top', others → NULL
    assert_eq!(outcome.rows[0].get("t"), Some(&Value::Null));
    assert_eq!(outcome.rows[1].get("t"), Some(&Value::Text("top".into())));
    assert_eq!(outcome.rows[2].get("t"), Some(&Value::Null));
}

#[test]
fn case_when_nested_inside_then_branch() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    // `CASE WHEN active THEN CASE WHEN score > 20 THEN 'active-high' ELSE 'active-low' END ELSE 'inactive' END`
    let outcome = execute_sql(
        "SELECT account_id, CASE WHEN active THEN CASE WHEN score > 20 THEN 'active-high' ELSE 'active-low' END ELSE 'inactive' END AS label FROM Account ORDER BY account_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("nested CASE");
    // ada: active=true, score=10 → active-low
    // linus: active=false → inactive
    // grace: active=true, score=20 → active-low (score > 20 is false)
    assert_eq!(outcome.rows[0].get("label"), Some(&Value::Text("active-low".into())));
    assert_eq!(outcome.rows[1].get("label"), Some(&Value::Text("inactive".into())));
    assert_eq!(outcome.rows[2].get("label"), Some(&Value::Text("active-low".into())));
}

// ─── CAST(expr AS type) ──────────────────────────────────────────────────

// These tests require oak-sql builder support for `Expression::Cast`.
// Executor handles INTEGER / REAL / TEXT / BLOB target types; NULL input
// stays NULL regardless of target.

#[test]
fn cast_integer_to_text_and_back() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT CAST(account_id AS TEXT) AS id_text, CAST(score AS REAL) AS score_r FROM Account WHERE account_id = 1",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("CAST int→text, int→real");
    assert_eq!(outcome.rows[0].get("id_text"), Some(&Value::Text("1".into())));
    assert_eq!(outcome.rows[0].get("score_r"), Some(&Value::F64(10.0)));
}

#[test]
fn cast_text_to_integer_succeeds_for_valid_number() {
    let contract = contract();
    let mut records = BTreeMap::new();

    // Insert a row where tenant is a numeric string via parameter (lowering
    // does not accept real SQL integer literals, but CAST inside projection
    // should lower).
    execute_sql(
        "INSERT INTO Account (account_id, email, tenant, score, active) VALUES (?, ?, ?, ?, ?)",
        &[
            Value::I64(5),
            Value::Text("cast@test".into()),
            Value::Text("42".into()),
            Value::I64(0),
            Value::Bool(true),
        ],
        &contract,
        &[],
        &mut records,
    )
    .expect("insert for cast");

    let outcome = execute_sql(
        "SELECT CAST(tenant AS INTEGER) AS tenant_int FROM Account WHERE account_id = 5",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("CAST text→int");
    assert_eq!(outcome.rows[0].get("tenant_int"), Some(&Value::I64(42)));
}

#[test]
fn cast_text_to_integer_non_numeric_is_error() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    // tenant='t1' is not a valid integer; this should surface a diagnostic
    // rather than silently producing NULL.
    let err = execute_sql(
        "SELECT CAST(tenant AS INTEGER) AS ti FROM Account WHERE account_id = 1",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect_err("CAST non-numeric text to INTEGER must error");
    let msg = format!("{err}");
    assert!(msg.contains("CAST") || msg.contains("INTEGER") || msg.contains("failed"), "unexpected error: {msg}");
}

#[test]
fn cast_null_preserves_null() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);
    exec("UPDATE Account SET tenant = NULL WHERE account_id = 1", &[], &contract, &mut records);

    let outcome = execute_sql(
        "SELECT CAST(tenant AS TEXT) AS t FROM Account WHERE account_id = 1",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("CAST null");
    assert_eq!(outcome.rows[0].get("t"), Some(&Value::Null));
}

#[test]
fn cast_bool_to_integer_and_back() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let outcome = execute_sql(
        "SELECT account_id, CAST(active AS INTEGER) AS a_int FROM Account ORDER BY account_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("CAST bool→int");
    // ada active=true → 1, linus active=false → 0, grace active=true → 1
    assert_eq!(outcome.rows[0].get("a_int"), Some(&Value::I64(1)));
    assert_eq!(outcome.rows[1].get("a_int"), Some(&Value::I64(0)));
    assert_eq!(outcome.rows[2].get("a_int"), Some(&Value::I64(1)));
}

#[test]
fn unsupported_oak_sql_clauses_are_rejected_with_their_own_message() {
    let contract = contract();
    let mut records = BTreeMap::new();

    let cases = [
        (
            "INSERT INTO Account (account_id, email, tenant, score, active) SELECT DISTINCT account_id, email, tenant, score, active FROM Account",
            "DISTINCT",
        ),
        ("SELECT account_id FROM Account UNION SELECT account_id FROM Account", "UNION"),
        ("SELECT account_id FROM Account EXCEPT SELECT account_id FROM Account", "EXCEPT"),
    ];

    for (sql, needle) in cases {
        let error = execute_sql(sql, &[], &contract, &[], &mut records).expect_err("clause is not supported yet");
        let message = format!("{error}");
        assert!(message.contains(needle), "expected `{needle}` in error, got `{message}`");
    }
    assert!(records.is_empty(), "rejected statements must not publish rows");
}

const MULTI_ROW_SCHEMA: &str = r#"
table Account {
    @@account_id: i64,
    @email: utf8,
    score: i64,
}
"#;

fn multi_row_contract() -> ResolvedContract {
    resolved_contract_from_document(MULTI_ROW_SCHEMA).expect("multi-row schema resolves")
}

fn email_unique() -> TableUniqueConstraint {
    let constraint = UniqueConstraint::new("Account_email_key", vec!["email".to_owned()], true).expect("constraint shape");
    TableUniqueConstraint::new("Account", constraint).expect("table binding")
}

#[test]
fn insert_values_publishes_every_row_and_reports_the_row_count() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();

    let inserted = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10), (2, 'linus@test', 30), (3, 'grace@test', 20)",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("multi-row insert");
    assert_eq!(inserted.affected, 3);
    assert_eq!(inserted.rows.len(), 3);

    let rows = execute_sql("SELECT account_id, score FROM Account ORDER BY account_id", &[], &contract, &[], &mut records)
        .expect("read back")
        .rows;
    assert_eq!(ids(&rows), vec![1, 2, 3]);
    assert_eq!(rows[1].get("score"), Some(&Value::I64(30)));

    let single = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (4, 'ken@test', 40)",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("single-row insert");
    assert_eq!(single.affected, 1);
    assert_eq!(records.len(), 4);
}

#[test]
fn multi_row_insert_rejects_a_repeated_primary_key_without_publishing() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();

    let error = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10), (1, 'linus@test', 30)",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect_err("repeated primary key");
    match error {
        Error::UniqueViolation { constraint, columns, .. } => {
            assert_eq!(constraint, "Account_pkey");
            assert_eq!(columns, vec!["account_id".to_owned()]);
        }
        other => panic!("expected a unique violation, got {other}"),
    }
    assert!(records.is_empty(), "the statement must not publish its first row");
}

#[test]
fn multi_row_insert_rolls_back_when_a_row_conflicts_with_a_stored_row() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();
    execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10)",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("seed row");

    let error = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (2, 'linus@test', 30), (3, 'ada@test', 20)",
        &[],
        &contract,
        &[email_unique()],
        &mut records,
    )
    .expect_err("duplicate email");
    match error {
        Error::UniqueViolation { constraint, columns, .. } => {
            assert_eq!(constraint, "Account_email_key");
            assert_eq!(columns, vec!["email".to_owned()]);
        }
        other => panic!("expected a unique violation, got {other}"),
    }
    assert_eq!(records.len(), 1, "the stored row is untouched and row 2 was not published");
}

#[test]
fn multi_row_insert_rejects_a_unique_conflict_inside_the_statement() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();

    let error = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10), (2, 'ada@test', 30)",
        &[],
        &contract,
        &[email_unique()],
        &mut records,
    )
    .expect_err("duplicate email inside one statement");
    assert!(matches!(error, Error::UniqueViolation { .. }), "expected a unique violation, got {error}");
    assert!(records.is_empty(), "no row of the failed statement may be published");
}

// ─── INSERT … ON CONFLICT … DO NOTHING ─────────────────────────────────

#[test]
fn on_conflict_do_nothing_skips_a_row_that_collides_on_the_primary_key() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();
    execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10)",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("seed row");

    let outcome = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'other@test', 99) ON CONFLICT (account_id) DO NOTHING",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("conflict is skipped");

    assert_eq!(outcome.affected, 0);
    assert!(outcome.rows.is_empty());
    assert_eq!(records.len(), 1, "the conflicting row was not published");
    let rows = execute_sql("SELECT email, score FROM Account", &[], &contract, &[], &mut records).expect("read back").rows;
    assert_eq!(rows[0].get("email"), Some(&Value::Text("ada@test".into())));
    assert_eq!(rows[0].get("score"), Some(&Value::I64(10)));
}

#[test]
fn on_conflict_do_nothing_skips_only_the_colliding_rows_of_one_statement() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();
    execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10)",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("seed row");

    let outcome = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (2, 'linus@test', 30), (1, 'other@test', 99), (3, 'grace@test', 20) ON CONFLICT (account_id) DO NOTHING",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("only the colliding row is skipped");

    assert_eq!(outcome.affected, 2);
    assert_eq!(ids(&outcome.rows), vec![2, 3]);
    assert_eq!(records.len(), 3);
}

#[test]
fn on_conflict_do_nothing_without_a_collision_inserts_every_row() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();

    let outcome = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10), (2, 'linus@test', 30) ON CONFLICT (account_id) DO NOTHING",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("no conflict to skip");

    assert_eq!(outcome.affected, 2);
    assert_eq!(records.len(), 2);
}

#[test]
fn on_conflict_do_nothing_detects_a_collision_inside_the_same_statement() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();

    let outcome = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10), (1, 'linus@test', 30) ON CONFLICT (account_id) DO NOTHING",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("the repeated primary key is skipped instead of rejected");

    assert_eq!(outcome.affected, 1);
    assert_eq!(records.len(), 1);
    let rows = execute_sql("SELECT email FROM Account", &[], &contract, &[], &mut records).expect("read back").rows;
    assert_eq!(rows[0].get("email"), Some(&Value::Text("ada@test".into())));
}

#[test]
fn on_conflict_targets_a_declared_unique_constraint() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();
    execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10)",
        &[],
        &contract,
        &[email_unique()],
        &mut records,
    )
    .expect("seed row");

    let outcome = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (2, 'ada@test', 30) ON CONFLICT (email) DO NOTHING",
        &[],
        &contract,
        &[email_unique()],
        &mut records,
    )
    .expect("the unique email collision is skipped");

    assert_eq!(outcome.affected, 0);
    assert_eq!(records.len(), 1);
}

#[test]
fn on_conflict_rejects_a_target_no_unique_constraint_covers() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();

    let error = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10) ON CONFLICT (score) DO NOTHING",
        &[],
        &contract,
        &[email_unique()],
        &mut records,
    )
    .expect_err("`score` is not unique");
    let message = format!("{error}");
    assert!(message.contains("not backed by a unique constraint"), "unexpected error: {message}");
    assert!(message.contains("score"), "the rejected target must be named: {message}");
    assert!(records.is_empty(), "a rejected clause must not publish rows");
}

#[test]
fn on_conflict_rejects_an_unknown_target_column() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();

    let error = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10) ON CONFLICT (missing) DO NOTHING",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect_err("unknown conflict target");
    assert!(format!("{error}").contains("unknown field `missing`"), "unexpected error: {error}");
    assert!(records.is_empty());
}

// ─── INSERT … ON CONFLICT … DO UPDATE ──────────────────────────────────

#[test]
fn on_conflict_do_update_replaces_the_conflicting_row() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();
    execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10)",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("seed row");

    let outcome = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'other@test', 99) ON CONFLICT (account_id) DO UPDATE SET score = 42",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("the conflicting row is updated");

    assert_eq!(outcome.affected, 1);
    assert_eq!(outcome.rows.len(), 1);
    assert_eq!(outcome.rows[0].get("score"), Some(&Value::I64(42)));
    assert_eq!(records.len(), 1, "no second row is published");
    let rows = execute_sql("SELECT email, score FROM Account", &[], &contract, &[], &mut records).expect("read back").rows;
    assert_eq!(rows[0].get("email"), Some(&Value::Text("ada@test".into())));
    assert_eq!(rows[0].get("score"), Some(&Value::I64(42)));
}

#[test]
fn on_conflict_do_update_inserts_a_row_that_does_not_conflict() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();
    execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10)",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("seed row");

    let outcome = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (2, 'linus@test', 30) ON CONFLICT (account_id) DO UPDATE SET score = 42",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("no conflict");

    assert_eq!(outcome.affected, 1);
    assert_eq!(records.len(), 2);
    let rows = execute_sql("SELECT score FROM Account WHERE account_id = 2", &[], &contract, &[], &mut records)
        .expect("read back")
        .rows;
    assert_eq!(rows[0].get("score"), Some(&Value::I64(30)), "the inserted row keeps its own values");
}

#[test]
fn on_conflict_do_update_mixes_updated_and_inserted_rows() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();
    exec(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10), (2, 'linus@test', 30)",
        &[],
        &contract,
        &mut records,
    );

    let outcome = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (2, 'linus2@test', 31), (3, 'grace@test', 20) ON CONFLICT (account_id) DO UPDATE SET score = 99",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("one update and one insert");

    assert_eq!(outcome.affected, 2);
    assert_eq!(ids(&outcome.rows), vec![2, 3]);
    assert_eq!(records.len(), 3);
    let rows = execute_sql("SELECT account_id, score FROM Account ORDER BY account_id", &[], &contract, &[], &mut records)
        .expect("read back")
        .rows;
    assert_eq!(rows[1].get("score"), Some(&Value::I64(99)), "row 2 was updated");
    assert_eq!(rows[2].get("score"), Some(&Value::I64(20)), "row 3 was inserted");
}

#[test]
fn on_conflict_do_update_where_selects_whether_the_row_is_updated() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();
    exec("INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10)", &[], &contract, &mut records);

    // Predicate is read on the conflicting row (score 10), so the update runs.
    let applied = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'other@test', 99) ON CONFLICT (account_id) DO UPDATE SET score = 20 WHERE score = 10",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("the predicate holds");
    assert_eq!(applied.affected, 1);

    // The conflicting row now has score 20, so the same predicate drops the update.
    let dropped = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'other@test', 99) ON CONFLICT (account_id) DO UPDATE SET score = 30 WHERE score = 10",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("the predicate drops the update");
    assert_eq!(dropped.affected, 0);
    assert!(dropped.rows.is_empty());

    let rows = execute_sql("SELECT score FROM Account", &[], &contract, &[], &mut records).expect("read back").rows;
    assert_eq!(rows[0].get("score"), Some(&Value::I64(20)), "the rejected update left the row untouched");
}

#[test]
fn on_conflict_do_update_reads_the_conflicting_rows_own_columns() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();
    exec("INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10)", &[], &contract, &mut records);

    execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'other@test', 99) ON CONFLICT (account_id) DO UPDATE SET score = score + 5",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("self-referencing assignment");

    let rows = execute_sql("SELECT score FROM Account", &[], &contract, &[], &mut records).expect("read back").rows;
    assert_eq!(rows[0].get("score"), Some(&Value::I64(15)), "the assignment reads the stored row, not the incoming one");
}

#[test]
fn on_conflict_do_update_targets_a_unique_column_and_keeps_the_stored_key() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();
    exec("INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10)", &[], &contract, &mut records);

    let outcome = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (2, 'ada@test', 99) ON CONFLICT (email) DO UPDATE SET score = 30",
        &[],
        &contract,
        &[email_unique()],
        &mut records,
    )
    .expect("the email collision updates the stored row");

    assert_eq!(outcome.affected, 1);
    assert_eq!(ids(&outcome.rows), vec![1], "the returned row is the stored one, not the incoming primary key");
    assert_eq!(records.len(), 1, "the incoming primary key is not stored as a new row");
    let rows = execute_sql("SELECT account_id, score FROM Account", &[], &contract, &[], &mut records).expect("read back").rows;
    assert_eq!(rows[0].get("account_id"), Some(&Value::I64(1)));
    assert_eq!(rows[0].get("score"), Some(&Value::I64(30)));
}

#[test]
fn on_conflict_do_update_rolls_back_when_the_updated_row_breaks_a_unique_rule() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();
    exec(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10), (2, 'linus@test', 30)",
        &[],
        &contract,
        &mut records,
    );

    let error = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'other@test', 99) ON CONFLICT (account_id) DO UPDATE SET email = 'linus@test'",
        &[],
        &contract,
        &[email_unique()],
        &mut records,
    )
    .expect_err("the updated row would duplicate the other row's email");
    assert!(matches!(error, Error::UniqueViolation { .. }), "expected a unique violation, got {error}");
    assert_eq!(records.len(), 2);
    let rows = execute_sql("SELECT email FROM Account WHERE account_id = 1", &[], &contract, &[], &mut records)
        .expect("read back")
        .rows;
    assert_eq!(rows[0].get("email"), Some(&Value::Text("ada@test".into())), "the stored row is untouched");
}

#[test]
fn on_conflict_do_update_rejects_an_excluded_reference_in_assignments() {
    let error = prepare_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10) ON CONFLICT (account_id) DO UPDATE SET score = excluded.score",
    )
    .expect_err("`excluded` is not modeled");
    let message = format!("{error}");
    assert!(message.contains("excluded"), "unexpected error: {message}");
    assert!(message.contains("does not model"), "the rejection must say why: {message}");
}

#[test]
fn on_conflict_do_update_rejects_an_excluded_reference_in_where() {
    let error = prepare_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10) ON CONFLICT (account_id) DO UPDATE SET score = 20 WHERE excluded.score > 5",
    )
    .expect_err("`excluded` is not modeled");
    let message = format!("{error}");
    assert!(message.contains("excluded"), "unexpected error: {message}");
}

#[test]
fn on_conflict_do_update_rejects_a_primary_key_assignment() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();
    exec("INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10)", &[], &contract, &mut records);

    let error = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'other@test', 99) ON CONFLICT (account_id) DO UPDATE SET account_id = 7",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect_err("primary key cannot be reassigned");
    assert!(format!("{error}").contains("primary key"), "unexpected error: {error}");
    assert_eq!(records.len(), 1);
}

#[test]
fn on_conflict_do_update_rejects_an_unknown_assignment_column() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();
    exec("INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10)", &[], &contract, &mut records);

    let error = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'other@test', 99) ON CONFLICT (account_id) DO UPDATE SET missing = 1",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect_err("unknown assignment column");
    assert!(format!("{error}").contains("unknown field `missing`"), "unexpected error: {error}");
    assert_eq!(records.len(), 1);
}

#[test]
fn on_conflict_do_update_rejects_a_target_no_unique_constraint_covers() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();

    let error = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10) ON CONFLICT (score) DO UPDATE SET score = 20",
        &[],
        &contract,
        &[email_unique()],
        &mut records,
    )
    .expect_err("`score` is not unique");
    assert!(format!("{error}").contains("not backed by a unique constraint"), "unexpected error: {error}");
    assert!(records.is_empty());
}

// ─── INSERT … RETURNING ────────────────────────────────────────────────

#[test]
fn insert_returning_hands_back_the_inserted_row() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();

    let outcome = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10) RETURNING account_id, email, score",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("insert with returning");

    assert_eq!(outcome.affected, 1);
    assert_eq!(outcome.rows.len(), 1);
    assert_eq!(outcome.rows[0].len(), 3, "only the named columns are returned");
    assert_eq!(outcome.rows[0].get("account_id"), Some(&Value::I64(1)));
    assert_eq!(outcome.rows[0].get("email"), Some(&Value::Text("ada@test".into())));
    assert_eq!(outcome.rows[0].get("score"), Some(&Value::I64(10)));
}

#[test]
fn insert_returning_hands_back_every_row_in_statement_order() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();

    let outcome = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10), (2, 'linus@test', 30) RETURNING account_id, score",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("multi-row insert with returning");

    assert_eq!(outcome.affected, 2);
    assert_eq!(ids(&outcome.rows), vec![1, 2]);
    assert_eq!(outcome.rows[1].get("score"), Some(&Value::I64(30)));
}

#[test]
fn insert_returning_projects_aliases_and_expressions() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();

    let outcome = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10) RETURNING email AS who, CAST(score AS TEXT) AS label, CASE WHEN score >= 20 THEN 'high' ELSE 'low' END AS band",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("returning aliases and expressions");

    let row = &outcome.rows[0];
    assert_eq!(row.get("who"), Some(&Value::Text("ada@test".into())));
    assert_eq!(row.get("label"), Some(&Value::Text("10".into())));
    assert_eq!(row.get("band"), Some(&Value::Text("low".into())));
}

#[test]
fn insert_returning_star_returns_the_written_row() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();

    let outcome = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10) RETURNING *",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("returning star");

    assert_eq!(outcome.rows.len(), 1);
    assert_eq!(outcome.rows[0].len(), 3);
    assert_eq!(outcome.rows[0].get("email"), Some(&Value::Text("ada@test".into())));
}

#[test]
fn insert_returning_returns_the_row_updated_by_on_conflict() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();
    exec("INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10)", &[], &contract, &mut records);

    let outcome = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'other@test', 99) ON CONFLICT (account_id) DO UPDATE SET score = 42 RETURNING account_id, score",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("returning an updated row");

    assert_eq!(outcome.affected, 1);
    assert_eq!(ids(&outcome.rows), vec![1]);
    assert_eq!(outcome.rows[0].get("score"), Some(&Value::I64(42)));
}

#[test]
fn insert_returning_omits_rows_the_conflict_clause_skipped() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();
    exec("INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10)", &[], &contract, &mut records);

    let outcome = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'other@test', 99) ON CONFLICT (account_id) DO NOTHING RETURNING account_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("the skipped row is not returned");

    assert_eq!(outcome.affected, 0);
    assert!(outcome.rows.is_empty());
}

#[test]
fn insert_select_returning_hands_back_the_produced_rows() {
    let contract = insert_select_contract();
    let mut records = BTreeMap::new();
    seed_sources(&contract, &mut records);

    let outcome = execute_sql(
        "INSERT INTO Target (target_id, email, score) SELECT source_id, email, score FROM Source ORDER BY source_id RETURNING target_id, score",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("insert select with returning");

    assert_eq!(outcome.affected, 3);
    let ids: Vec<i64> = outcome
        .rows
        .iter()
        .map(|row| match row.get("target_id") {
            Some(Value::I64(id)) => *id,
            other => panic!("unexpected target_id {other:?}"),
        })
        .collect();
    assert_eq!(ids, vec![1, 2, 3]);
}

#[test]
fn insert_returning_rejects_an_unknown_column_without_publishing() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();

    let error = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10) RETURNING missing",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect_err("unknown returning column");
    assert!(format!("{error}").contains("unknown field `missing`"), "unexpected error: {error}");
    assert!(records.is_empty(), "a rejected clause must not publish rows");
}

#[test]
fn insert_returning_rejects_an_unknown_column_inside_an_expression() {
    let contract = multi_row_contract();
    let mut records = BTreeMap::new();

    let error = execute_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10) RETURNING CASE WHEN missing > 1 THEN 'x' ELSE 'y' END AS band",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect_err("unknown column inside a returning expression");
    assert!(format!("{error}").contains("unknown field `missing`"), "unexpected error: {error}");
    assert!(records.is_empty());
}

#[test]
fn insert_returning_rejects_an_aggregate_function() {
    let error = prepare_sql(
        "INSERT INTO Account (account_id, email, score) VALUES (1, 'ada@test', 10) RETURNING COUNT(*)",
    )
    .expect_err("aggregates have no per-row meaning in RETURNING");
    assert!(format!("{error}").contains("aggregate"), "unexpected error: {error}");
}

const INSERT_SELECT_SCHEMA: &str = r#"
table Source {
    @@source_id: i64,
    email: utf8,
    score: i64,
}

table Target {
    @@target_id: i64,
    email: utf8,
    score: i64,
}

table Mirror {
    @@mirror_id: i64,
    email: utf8,
    score: i64,
}

table Event {
    @@event_id: i64,
    happened_on: date,
}
"#;

fn insert_select_contract() -> ResolvedContract {
    resolved_contract_from_document(INSERT_SELECT_SCHEMA).expect("insert-select schema resolves")
}

fn seed_sources(contract: &ResolvedContract, records: &mut BTreeMap<String, Vec<u8>>) {
    exec(
        "INSERT INTO Source (source_id, email, score) VALUES (1, 'ada@test', 10), (2, 'linus@test', 30), (3, 'grace@test', 20)",
        &[],
        contract,
        records,
    );
}

fn target_email_unique() -> TableUniqueConstraint {
    let constraint = UniqueConstraint::new("Target_email_key", vec!["email".to_owned()], true).expect("constraint shape");
    TableUniqueConstraint::new("Target", constraint).expect("table binding")
}

fn column(rows: &[BTreeMap<String, Value>], name: &str) -> Vec<Value> {
    rows.iter().map(|row| row.get(name).cloned().unwrap_or(Value::Null)).collect()
}

#[test]
fn insert_select_lowers_into_a_write_op_carrying_its_read_pipeline() {
    let stmt = prepare_sql(
        "INSERT INTO Target (target_id, email, score) SELECT source_id, email, score FROM Source WHERE score > 10",
    )
    .expect("prepare");
    assert_eq!(stmt.query_kind, QueryKind::Write);
    assert!(stmt.ops.is_empty(), "a write statement carries no top-level read pipeline");
    assert_eq!(stmt.read_set, vec!["Source".to_owned()]);
    assert_eq!(stmt.write_set, vec!["Target".to_owned()]);

    match stmt.write_ops.as_slice() {
        [WriteOp::InsertFromQuery { table, columns, source, outputs, .. }] => {
            assert_eq!(table, "Target");
            assert_eq!(columns, &vec!["target_id".to_owned(), "email".to_owned(), "score".to_owned()]);
            assert_eq!(
                outputs.as_deref(),
                Some(&["source_id".to_owned(), "email".to_owned(), "score".to_owned()][..])
            );
            assert!(matches!(source.last(), Some(QueryOp::Collect)), "source must end in Collect");
        }
        other => panic!("unexpected write ops {other:?}"),
    }
}

#[test]
fn insert_select_publishes_the_rows_of_a_single_table_source() {
    let contract = insert_select_contract();
    let mut records = BTreeMap::new();
    seed_sources(&contract, &mut records);

    let inserted = execute_sql(
        "INSERT INTO Target (target_id, email, score) SELECT source_id, email, score FROM Source WHERE score >= 20",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("insert select");
    assert_eq!(inserted.affected, 2);
    assert_eq!(inserted.rows.len(), 2);

    let rows = execute_sql("SELECT target_id, score FROM Target ORDER BY target_id", &[], &contract, &[], &mut records)
        .expect("read back")
        .rows;
    assert_eq!(column(&rows, "target_id"), vec![Value::I64(2), Value::I64(3)]);
    assert_eq!(column(&rows, "score"), vec![Value::I64(30), Value::I64(20)]);
}

#[test]
fn insert_select_projects_constant_columns_into_the_target() {
    let contract = insert_select_contract();
    let mut records = BTreeMap::new();
    seed_sources(&contract, &mut records);

    // Constant columns need distinct aliases: unnamed expressions share one
    // placeholder name in the read pipeline. Aliases deliberately differ from
    // the target column names so the mapping is proven to be positional.
    let inserted = execute_sql(
        "INSERT INTO Target (target_id, email, score) SELECT 900 AS id_col, 'copy@test' AS mail_col, 7 AS score_col FROM Source WHERE source_id = 1",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("constant select");
    assert_eq!(inserted.affected, 1);

    let rows = execute_sql("SELECT target_id, email, score FROM Target", &[], &contract, &[], &mut records)
        .expect("read back")
        .rows;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("target_id"), Some(&Value::I64(900)));
    assert_eq!(rows[0].get("email"), Some(&Value::Text("copy@test".into())));
    assert_eq!(rows[0].get("score"), Some(&Value::I64(7)));
}

#[test]
fn insert_select_star_uses_the_catalog_column_order() {
    let contract = insert_select_contract();
    let mut records = BTreeMap::new();
    seed_sources(&contract, &mut records);
    exec(
        "INSERT INTO Target (target_id, email, score) SELECT source_id, email, score FROM Source WHERE source_id = 2",
        &[],
        &contract,
        &mut records,
    );

    // `SELECT *` with an omitted target column list maps the source fields to
    // the target fields in catalog declaration order.
    let inserted =
        execute_sql("INSERT INTO Mirror SELECT * FROM Target", &[], &contract, &[], &mut records).expect("insert select star");
    assert_eq!(inserted.affected, 1);

    let rows = execute_sql("SELECT mirror_id, email, score FROM Mirror", &[], &contract, &[], &mut records)
        .expect("read back")
        .rows;
    assert_eq!(rows[0].get("mirror_id"), Some(&Value::I64(2)));
    assert_eq!(rows[0].get("email"), Some(&Value::Text("linus@test".into())));
    assert_eq!(rows[0].get("score"), Some(&Value::I64(30)));
}

#[test]
fn insert_select_rejects_a_source_column_count_mismatch() {
    let contract = insert_select_contract();
    let mut records = BTreeMap::new();
    seed_sources(&contract, &mut records);

    let error = execute_sql(
        "INSERT INTO Target (target_id, score) SELECT source_id, email, score FROM Source",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect_err("three source columns for two target columns");
    let message = format!("{error}");
    assert!(message.contains("column count mismatch"), "unexpected error `{message}`");
    assert!(!records.is_empty(), "the source rows must stay untouched");
    assert!(execute_sql("SELECT target_id FROM Target", &[], &contract, &[], &mut records).unwrap().rows.is_empty());
}

#[test]
fn insert_select_rejects_a_value_the_target_column_cannot_hold() {
    let contract = insert_select_contract();
    let mut records = BTreeMap::new();
    seed_sources(&contract, &mut records);

    // `Event.happened_on` is a date column and the source value is text that is
    // not an ISO date, so the shared scalar coercion rejects the row.
    let error = execute_sql(
        "INSERT INTO Event (event_id, happened_on) SELECT source_id, email FROM Source",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect_err("text is not a date");
    assert!(matches!(error, Error::Scalar { .. }), "expected a scalar error, got {error}");
    assert!(
        execute_sql("SELECT event_id FROM Event", &[], &contract, &[], &mut records).unwrap().rows.is_empty(),
        "no row of the failed statement may be published"
    );
}

#[test]
fn insert_select_publishes_nothing_when_a_row_violates_a_unique_constraint() {
    let contract = insert_select_contract();
    let mut records = BTreeMap::new();
    exec(
        "INSERT INTO Source (source_id, email, score) VALUES (1, 'shared@test', 10), (2, 'shared@test', 30), (3, 'other@test', 20)",
        &[],
        &contract,
        &mut records,
    );

    let error = execute_sql(
        "INSERT INTO Target (target_id, email, score) SELECT source_id, email, score FROM Source",
        &[],
        &contract,
        &[target_email_unique()],
        &mut records,
    )
    .expect_err("the second source row repeats the first email");
    match error {
        Error::UniqueViolation { table, constraint, columns } => {
            assert_eq!(table, "Target");
            assert_eq!(constraint, "Target_email_key");
            assert_eq!(columns, vec!["email".to_owned()]);
        }
        other => panic!("expected a unique violation, got {other}"),
    }
    assert!(
        execute_sql("SELECT target_id FROM Target", &[], &contract, &[], &mut records).unwrap().rows.is_empty(),
        "the statement must not publish its first row"
    );
}

#[test]
fn insert_select_from_the_target_table_reads_the_pre_statement_snapshot() {
    let contract = insert_select_contract();
    let mut records = BTreeMap::new();
    seed_sources(&contract, &mut records);

    let inserted = execute_sql(
        "INSERT INTO Source (source_id, email, score) SELECT source_id, email, score FROM Source",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("self-referencing insert select");
    assert_eq!(inserted.affected, 3);

    let rows = execute_sql("SELECT source_id, score FROM Source ORDER BY source_id", &[], &contract, &[], &mut records)
        .expect("read back")
        .rows;
    assert_eq!(column(&rows, "source_id"), vec![Value::I64(1), Value::I64(2), Value::I64(3)]);
    assert_eq!(column(&rows, "score"), vec![Value::I64(10), Value::I64(30), Value::I64(20)]);
}

#[test]
fn insert_select_rejects_sources_the_read_pipeline_cannot_shape() {
    let contract = insert_select_contract();
    let mut records = BTreeMap::new();
    seed_sources(&contract, &mut records);

    let cases = [
        (
            "INSERT INTO Target (target_id, email, score) SELECT COUNT(*) AS n, 'x@test', 0 FROM Source",
            "aggregate functions",
        ),
        (
            "INSERT INTO Target (target_id, email, score) SELECT score, email, source_id FROM Source GROUP BY score",
            "GROUP BY",
        ),
        (
            "INSERT INTO Target (target_id, email, score) SELECT source_id, email, email FROM Source",
            "appears more than once",
        ),
    ];

    for (sql, needle) in cases {
        let error = execute_sql(sql, &[], &contract, &[], &mut records).expect_err("source shape is rejected");
        let message = format!("{error}");
        assert!(message.contains(needle), "expected `{needle}` in error, got `{message}`");
    }
    assert!(
        execute_sql("SELECT target_id FROM Target", &[], &contract, &[], &mut records).unwrap().rows.is_empty(),
        "rejected statements must not publish rows"
    );
}

// ─── table aliases and self joins ──────────────────────────────────────

#[test]
fn aliased_from_qualifies_every_output_column() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let rows = execute_sql("SELECT * FROM Account AS a WHERE a.account_id = 1", &[], &contract, &[], &mut records)
        .expect("aliased scan")
        .rows;
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.get("a.email"), Some(&Value::Text("ada@example.test".into())));
    assert_eq!(row.get("a.score"), Some(&Value::I64(10)));
    assert!(row.get("email").is_none(), "an aliased source must not leak bare column names");
}

#[test]
fn aliased_self_join_reads_both_occurrences() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    // Accounts 1 and 2 share tenant t1, so `a.account_id < b.account_id` keeps
    // exactly the (1, 2) pair. Both sides are the same table, told apart only
    // by their aliases.
    let rows = execute_sql(
        "SELECT a.account_id AS left_id, b.account_id AS right_id FROM Account AS a \
         JOIN Account AS b ON a.tenant = b.tenant WHERE a.account_id < b.account_id ORDER BY a.account_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("self join")
    .rows;
    assert_eq!(column(&rows, "left_id"), vec![Value::I64(1)]);
    assert_eq!(column(&rows, "right_id"), vec![Value::I64(2)]);
}

#[test]
fn aliased_join_prefixes_each_side_with_its_own_alias() {
    let contract = left_join_contract();
    let mut records = BTreeMap::new();
    seed_left_join(&contract, &mut records);

    let rows = execute_sql(
        "SELECT * FROM Account AS a LEFT JOIN Project AS p ON a.account_id = p.owner_id ORDER BY a.account_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("aliased left join")
    .rows;
    assert_eq!(rows.len(), 4);
    // Account 3 has no project: its right-hand columns are padded with NULL.
    let padded = rows.iter().find(|row| row.get("a.account_id") == Some(&Value::I64(3))).expect("padded row");
    assert_eq!(padded.get("p.name"), Some(&Value::Null));
    assert_eq!(padded.get("a.email"), Some(&Value::Text("grace@test".into())));
}

#[test]
fn aliased_statements_reject_unqualified_and_unknown_references() {
    let cases = [
        ("SELECT account_id FROM Account AS a", "must be qualified"),
        ("SELECT c.account_id FROM Account AS a", "not a row source"),
        ("SELECT * FROM Account AS a JOIN Account AS a ON a.account_id = a.account_id", "referenced more than once"),
        ("SELECT * FROM Account JOIN Account ON Account.account_id = Account.account_id", "referenced more than once"),
    ];

    for (sql, needle) in cases {
        let error = prepare_sql(sql).expect_err("the reference is ambiguous or unknown");
        let message = format!("{error}");
        assert!(message.contains(needle), "expected `{needle}` in error, got `{message}`");
    }
}

// ─── derived tables ────────────────────────────────────────────────────

#[test]
fn derived_table_scans_its_sub_pipeline_and_exposes_the_alias() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed(&contract, &mut records);

    let stmt = prepare_sql("SELECT s.account_id FROM (SELECT account_id, email FROM Account WHERE score >= 20) AS s")
        .expect("prepare derived table");
    assert_eq!(stmt.read_set, vec!["Account".to_owned()], "a derived table still reads its base table");

    // The inner pipeline runs first (Scan / Filter / Project), then the outer
    // statement sees the derived rows as `s.*`.
    let rows = execute_sql(
        "SELECT s.account_id AS id, s.email AS mail \
         FROM (SELECT account_id, email FROM Account WHERE score >= 20) AS s ORDER BY s.account_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("derived table select")
    .rows;
    assert_eq!(column(&rows, "id"), vec![Value::I64(2), Value::I64(3)]);
    assert_eq!(
        column(&rows, "mail"),
        vec![Value::Text("linus@example.test".into()), Value::Text("grace@example.test".into())]
    );
}

#[test]
fn derived_table_joins_as_a_nested_pipeline() {
    let contract = left_join_contract();
    let mut records = BTreeMap::new();
    seed_left_join(&contract, &mut records);

    // Project 10 is filtered out, so Account 1 only matches `p2`.
    let rows = execute_sql(
        "SELECT a.account_id AS id, p.name AS project \
         FROM Account AS a LEFT JOIN (SELECT owner_id, name FROM Project WHERE project_id > 10) AS p \
         ON a.account_id = p.owner_id ORDER BY a.account_id",
        &[],
        &contract,
        &[],
        &mut records,
    )
    .expect("derived left join")
    .rows;

    assert_eq!(column(&rows, "id"), vec![Value::I64(1), Value::I64(2), Value::I64(3)]);
    assert_eq!(
        column(&rows, "project"),
        vec![Value::Text("p2".into()), Value::Text("p3".into()), Value::Null],
        "Account 3 has no project and must be padded with NULL"
    );
}

#[test]
fn derived_tables_require_an_alias_and_qualified_columns() {
    let unaliased = prepare_sql("SELECT account_id FROM (SELECT account_id FROM Account)")
        .expect_err("a derived table without an alias has no referenceable name");
    assert!(format!("{unaliased}").contains("requires an alias"), "unexpected error: {unaliased}");

    let unqualified = prepare_sql("SELECT account_id FROM (SELECT account_id FROM Account) AS s")
        .expect_err("an aliased derived table only exposes its alias");
    assert!(format!("{unqualified}").contains("must be qualified"), "unexpected error: {unqualified}");

    let unknown = prepare_sql("SELECT t.account_id FROM (SELECT account_id FROM Account) AS s")
        .expect_err("an unknown derived alias must not resolve");
    assert!(format!("{unknown}").contains("not a row source"), "unexpected error: {unknown}");
}
