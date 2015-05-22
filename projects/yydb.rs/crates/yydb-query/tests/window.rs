//! Window functions: partitioning, ranking, aggregate frames and rejections.

use std::collections::BTreeMap;

use vos::types::ResolvedContract;
use yydb_query::{execute_sql, prepare_sql, resolved_contract_from_document};
use yydb_types::Value;

const SCHEMA: &str = r#"
table Sale {
    @@sale_id: i64,
    region: utf8,
    amount: i64,
}
"#;

fn contract() -> ResolvedContract {
    resolved_contract_from_document(SCHEMA).expect("sale schema resolves")
}

fn exec(sql: &str, contract: &ResolvedContract, records: &mut BTreeMap<String, Vec<u8>>) {
    execute_sql(sql, &[], contract, &[], records).expect("sql executes");
}

/// Run one query and return its rows.
fn rows(sql: &str, contract: &ResolvedContract, records: &mut BTreeMap<String, Vec<u8>>) -> Vec<BTreeMap<String, Value>> {
    execute_sql(sql, &[], contract, &[], records).expect("window query executes").rows
}

/// One column of one row, panicking on a missing column.
fn cell(row: &BTreeMap<String, Value>, name: &str) -> Value {
    row.get(name).cloned().unwrap_or_else(|| panic!("missing column `{name}` in {row:?}"))
}

/// Scores with ties: `east` holds 10, 20 and 20, `west` holds 5 and 15.
fn seed_ties(contract: &ResolvedContract, records: &mut BTreeMap<String, Vec<u8>>) {
    exec("INSERT INTO Sale (sale_id, region, amount) VALUES (1, 'east', 10)", contract, records);
    exec("INSERT INTO Sale (sale_id, region, amount) VALUES (2, 'east', 20)", contract, records);
    exec("INSERT INTO Sale (sale_id, region, amount) VALUES (3, 'east', 20)", contract, records);
    exec("INSERT INTO Sale (sale_id, region, amount) VALUES (4, 'west', 5)", contract, records);
    exec("INSERT INTO Sale (sale_id, region, amount) VALUES (5, 'west', 15)", contract, records);
}

// ─── ranking ────────────────────────────────────────────────────────────

#[test]
fn window_ranking_with_partition_and_ties() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed_ties(&contract, &mut records);

    let rows = rows(
        "SELECT sale_id, \
                ROW_NUMBER() OVER (PARTITION BY region ORDER BY amount) AS rn, \
                RANK() OVER (PARTITION BY region ORDER BY amount) AS rk, \
                DENSE_RANK() OVER (PARTITION BY region ORDER BY amount) AS dr \
         FROM Sale ORDER BY region, amount, sale_id",
        &contract,
        &mut records,
    );

    // east 10, east 20 (id 2), east 20 (id 3), west 5, west 15
    let expected = [(1_i64, 1_i64, 1_i64, 1_i64), (2, 2, 2, 2), (3, 3, 2, 2), (4, 1, 1, 1), (5, 2, 2, 2)];
    assert_eq!(rows.len(), expected.len());
    for (row, (id, rn, rk, dr)) in rows.iter().zip(expected) {
        assert_eq!(cell(row, "sale_id"), Value::I64(id));
        assert_eq!(cell(row, "rn"), Value::I64(rn));
        assert_eq!(cell(row, "rk"), Value::I64(rk));
        assert_eq!(cell(row, "dr"), Value::I64(dr));
    }
}

#[test]
fn window_order_by_uses_the_shared_null_ordering() {
    let contract = contract();
    let mut records = BTreeMap::new();
    exec("INSERT INTO Sale (sale_id, region, amount) VALUES (1, 'east', NULL)", &contract, &mut records);
    exec("INSERT INTO Sale (sale_id, region, amount) VALUES (2, 'east', 10)", &contract, &mut records);
    exec("INSERT INTO Sale (sale_id, region, amount) VALUES (3, 'east', 20)", &contract, &mut records);

    let rows = rows(
        "SELECT sale_id, \
                ROW_NUMBER() OVER (PARTITION BY region ORDER BY amount) AS rn, \
                SUM(amount) OVER (PARTITION BY region ORDER BY amount) AS running \
         FROM Sale ORDER BY sale_id",
        &contract,
        &mut records,
    );

    // `NULL` sorts first, exactly as `QueryOp::Sort` orders it.
    assert_eq!(cell(&rows[0], "rn"), Value::I64(1));
    assert_eq!(cell(&rows[0], "running"), Value::Null);
    assert_eq!(cell(&rows[1], "rn"), Value::I64(2));
    assert_eq!(cell(&rows[1], "running"), Value::I64(10));
    assert_eq!(cell(&rows[2], "rn"), Value::I64(3));
    assert_eq!(cell(&rows[2], "running"), Value::I64(30));
}

// ─── default frames ─────────────────────────────────────────────────────

#[test]
fn window_cumulative_sum_stops_at_the_peer_group() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed_ties(&contract, &mut records);

    let rows = rows(
        "SELECT sale_id, \
                SUM(amount) OVER (PARTITION BY region ORDER BY amount) AS running, \
                COUNT(amount) OVER (PARTITION BY region ORDER BY amount) AS seen \
         FROM Sale ORDER BY region, amount, sale_id",
        &contract,
        &mut records,
    );

    // A tie group is read as one frame end, so both `east` 20 rows see 10+20+20.
    let expected = [(1_i64, 10_i64, 1_i64), (2, 50, 3), (3, 50, 3), (4, 5, 1), (5, 20, 2)];
    for (row, (id, running, seen)) in rows.iter().zip(expected) {
        assert_eq!(cell(row, "sale_id"), Value::I64(id));
        assert_eq!(cell(row, "running"), Value::I64(running));
        assert_eq!(cell(row, "seen"), Value::I64(seen));
    }
}

#[test]
fn window_whole_partition_aggregates_without_order_by() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed_ties(&contract, &mut records);

    let rows = rows(
        "SELECT sale_id, \
                SUM(amount) OVER (PARTITION BY region) AS total, \
                AVG(amount) OVER (PARTITION BY region) AS mean, \
                COUNT(*) OVER (PARTITION BY region) AS n_all, \
                COUNT(amount) OVER (PARTITION BY region) AS n_value, \
                MIN(amount) OVER (PARTITION BY region) AS lowest, \
                MAX(amount) OVER (PARTITION BY region) AS highest \
         FROM Sale ORDER BY sale_id",
        &contract,
        &mut records,
    );

    // east: 10 + 20 + 20 = 50, west: 5 + 15 = 20
    let expected = [
        (1_i64, 50_i64, 50.0 / 3.0, 3_i64, 10_i64, 20_i64),
        (2, 50, 50.0 / 3.0, 3, 10, 20),
        (3, 50, 50.0 / 3.0, 3, 10, 20),
        (4, 20, 10.0, 2, 5, 15),
        (5, 20, 10.0, 2, 5, 15),
    ];
    for (row, (id, total, mean, n_all, lowest, highest)) in rows.iter().zip(expected) {
        assert_eq!(cell(row, "sale_id"), Value::I64(id));
        assert_eq!(cell(row, "total"), Value::I64(total));
        assert_eq!(cell(row, "mean"), Value::F64(mean));
        assert_eq!(cell(row, "n_all"), Value::I64(n_all));
        assert_eq!(cell(row, "n_value"), Value::I64(n_all));
        assert_eq!(cell(row, "lowest"), Value::I64(lowest));
        assert_eq!(cell(row, "highest"), Value::I64(highest));
    }
}

#[test]
fn window_single_row_partition() {
    let contract = contract();
    let mut records = BTreeMap::new();
    exec("INSERT INTO Sale (sale_id, region, amount) VALUES (1, 'north', 42)", &contract, &mut records);
    exec("INSERT INTO Sale (sale_id, region, amount) VALUES (2, 'east', 10)", &contract, &mut records);
    exec("INSERT INTO Sale (sale_id, region, amount) VALUES (3, 'east', 20)", &contract, &mut records);

    let rows = rows(
        "SELECT sale_id, \
                ROW_NUMBER() OVER (PARTITION BY region ORDER BY amount) AS rn, \
                SUM(amount) OVER (PARTITION BY region) AS total, \
                AVG(amount) OVER (PARTITION BY region) AS mean, \
                MIN(amount) OVER (PARTITION BY region) AS lowest \
         FROM Sale ORDER BY sale_id",
        &contract,
        &mut records,
    );

    assert_eq!(cell(&rows[0], "rn"), Value::I64(1));
    assert_eq!(cell(&rows[0], "total"), Value::I64(42));
    assert_eq!(cell(&rows[0], "mean"), Value::F64(42.0));
    assert_eq!(cell(&rows[0], "lowest"), Value::I64(42));
    assert_eq!(cell(&rows[1], "total"), Value::I64(30));
    assert_eq!(cell(&rows[2], "total"), Value::I64(30));
}

#[test]
fn window_all_null_partition() {
    let contract = contract();
    let mut records = BTreeMap::new();
    exec("INSERT INTO Sale (sale_id, region, amount) VALUES (1, 'east', NULL)", &contract, &mut records);
    exec("INSERT INTO Sale (sale_id, region, amount) VALUES (2, 'east', NULL)", &contract, &mut records);
    exec("INSERT INTO Sale (sale_id, region, amount) VALUES (3, 'west', 5)", &contract, &mut records);

    let rows = rows(
        "SELECT sale_id, \
                ROW_NUMBER() OVER (PARTITION BY region ORDER BY amount) AS rn, \
                RANK() OVER (PARTITION BY region ORDER BY amount) AS rk, \
                SUM(amount) OVER (PARTITION BY region) AS total, \
                COUNT(amount) OVER (PARTITION BY region) AS n_value, \
                MIN(amount) OVER (PARTITION BY region) AS lowest \
         FROM Sale ORDER BY sale_id",
        &contract,
        &mut records,
    );

    // Both `east` rows carry `NULL`, so they tie and every aggregate is `NULL`.
    assert_eq!(cell(&rows[0], "rn"), Value::I64(1));
    assert_eq!(cell(&rows[0], "rk"), Value::I64(1));
    assert_eq!(cell(&rows[0], "total"), Value::Null);
    assert_eq!(cell(&rows[0], "n_value"), Value::I64(0));
    assert_eq!(cell(&rows[0], "lowest"), Value::Null);
    assert_eq!(cell(&rows[1], "rn"), Value::I64(2));
    assert_eq!(cell(&rows[1], "rk"), Value::I64(1));
    assert_eq!(cell(&rows[1], "total"), Value::Null);
    assert_eq!(cell(&rows[1], "n_value"), Value::I64(0));

    assert_eq!(cell(&rows[2], "rn"), Value::I64(1));
    assert_eq!(cell(&rows[2], "total"), Value::I64(5));
    assert_eq!(cell(&rows[2], "n_value"), Value::I64(1));
    assert_eq!(cell(&rows[2], "lowest"), Value::I64(5));
}

#[test]
fn window_over_empty_table_yields_no_rows() {
    let contract = contract();
    let mut records = BTreeMap::new();

    let ranked = rows(
        "SELECT sale_id, ROW_NUMBER() OVER (PARTITION BY region ORDER BY amount) AS rn FROM Sale",
        &contract,
        &mut records,
    );
    assert!(ranked.is_empty());

    let summed = rows("SELECT SUM(amount) OVER (PARTITION BY region) AS total FROM Sale", &contract, &mut records);
    assert!(summed.is_empty());
}

// ─── explicit `ROWS` frames ─────────────────────────────────────────────

#[test]
fn window_rows_between_preceding_and_current_row() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed_ties(&contract, &mut records);

    let rows = rows(
        "SELECT sale_id, \
                SUM(amount) OVER (PARTITION BY region ORDER BY amount ROWS BETWEEN 1 PRECEDING AND CURRENT ROW) AS near, \
                COUNT(*) OVER (PARTITION BY region ORDER BY amount ROWS BETWEEN 1 PRECEDING AND CURRENT ROW) AS n \
         FROM Sale ORDER BY region, amount, sale_id",
        &contract,
        &mut records,
    );

    // east 10, 20, 20 and west 5, 15 — a row frame, so peers are separate rows.
    let expected = [(1_i64, 10_i64, 1_i64), (2, 30, 2), (3, 40, 2), (4, 5, 1), (5, 20, 2)];
    for (row, (id, near, n)) in rows.iter().zip(expected) {
        assert_eq!(cell(row, "sale_id"), Value::I64(id));
        assert_eq!(cell(row, "near"), Value::I64(near));
        assert_eq!(cell(row, "n"), Value::I64(n));
    }
}

#[test]
fn window_rows_between_current_row_and_following() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed_ties(&contract, &mut records);

    let rows = rows(
        "SELECT sale_id, \
                SUM(amount) OVER (PARTITION BY region ORDER BY amount ROWS BETWEEN CURRENT ROW AND 1 FOLLOWING) AS near, \
                COUNT(amount) OVER (PARTITION BY region ORDER BY amount ROWS BETWEEN CURRENT ROW AND 1 FOLLOWING) AS n \
         FROM Sale ORDER BY region, amount, sale_id",
        &contract,
        &mut records,
    );

    let expected = [(1_i64, 30_i64, 2_i64), (2, 40, 2), (3, 20, 1), (4, 20, 2), (5, 15, 1)];
    for (row, (id, near, n)) in rows.iter().zip(expected) {
        assert_eq!(cell(row, "sale_id"), Value::I64(id));
        assert_eq!(cell(row, "near"), Value::I64(near));
        assert_eq!(cell(row, "n"), Value::I64(n));
    }
}

#[test]
fn window_rows_unbounded_frames() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed_ties(&contract, &mut records);

    // `UNBOUNDED PRECEDING … CURRENT ROW` counts rows, so a tie group does not
    // extend the frame the way the default range frame does.
    let running_rows = rows(
        "SELECT sale_id, \
                SUM(amount) OVER (PARTITION BY region ORDER BY amount ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS running \
         FROM Sale ORDER BY region, amount, sale_id",
        &contract,
        &mut records,
    );
    let expected = [(1_i64, 10_i64), (2, 30), (3, 50), (4, 5), (5, 20)];
    for (row, (id, running)) in running_rows.iter().zip(expected) {
        assert_eq!(cell(row, "sale_id"), Value::I64(id));
        assert_eq!(cell(row, "running"), Value::I64(running));
    }

    // Both bounds unbounded is the whole partition.
    let whole = rows(
        "SELECT sale_id, \
                SUM(amount) OVER (PARTITION BY region ORDER BY amount ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING) AS total \
         FROM Sale ORDER BY sale_id",
        &contract,
        &mut records,
    );
    let expected = [(1_i64, 50_i64), (2, 50), (3, 50), (4, 20), (5, 20)];
    for (row, (id, total)) in whole.iter().zip(expected) {
        assert_eq!(cell(row, "sale_id"), Value::I64(id));
        assert_eq!(cell(row, "total"), Value::I64(total));
    }
}

#[test]
fn window_rows_frame_outside_the_partition_is_empty() {
    let contract = contract();
    let mut records = BTreeMap::new();
    seed_ties(&contract, &mut records);

    let rows = rows(
        "SELECT sale_id, \
                SUM(amount) OVER (PARTITION BY region ORDER BY amount ROWS BETWEEN 1 FOLLOWING AND 1 FOLLOWING) AS next, \
                COUNT(*) OVER (PARTITION BY region ORDER BY amount ROWS BETWEEN 1 FOLLOWING AND 1 FOLLOWING) AS n, \
                MIN(amount) OVER (PARTITION BY region ORDER BY amount ROWS BETWEEN 1 FOLLOWING AND 1 FOLLOWING) AS lowest \
         FROM Sale ORDER BY region, amount, sale_id",
        &contract,
        &mut records,
    );

    // The last row of each partition has no follower, so its frame is empty:
    // `COUNT(*)` is 0 and every other aggregate is `NULL`.
    let expected = [(1_i64, Value::I64(20), Value::I64(1)), (2, Value::I64(20), Value::I64(1))];
    for (row, (id, next, n)) in rows.iter().take(2).zip(expected) {
        assert_eq!(cell(row, "sale_id"), Value::I64(id));
        assert_eq!(cell(row, "next"), next);
        assert_eq!(cell(row, "n"), n);
    }
    assert_eq!(cell(&rows[2], "next"), Value::Null);
    assert_eq!(cell(&rows[2], "n"), Value::I64(0));
    assert_eq!(cell(&rows[2], "lowest"), Value::Null);

    assert_eq!(cell(&rows[3], "next"), Value::I64(15));
    assert_eq!(cell(&rows[4], "next"), Value::Null);
    assert_eq!(cell(&rows[4], "n"), Value::I64(0));
    assert_eq!(cell(&rows[4], "lowest"), Value::Null);
}

// ─── rejections ─────────────────────────────────────────────────────────

#[test]
fn window_range_frame_is_rejected() {
    let error = prepare_sql(
        "SELECT SUM(amount) OVER (PARTITION BY region ORDER BY amount RANGE BETWEEN 1 PRECEDING AND CURRENT ROW) AS s \
         FROM Sale",
    )
    .expect_err("a RANGE frame must be rejected");
    assert!(format!("{error}").contains("`RANGE` window frames"), "unexpected frame error: {error}");
}

#[test]
fn window_in_predicate_is_rejected() {
    let error = prepare_sql("SELECT sale_id FROM Sale WHERE ROW_NUMBER() OVER (ORDER BY amount) = 1")
        .expect_err("a window inside WHERE must be rejected");
    assert!(format!("{error}").contains("cannot appear in a predicate"), "unexpected predicate error: {error}");
}

#[test]
fn window_cannot_mix_with_group_by() {
    let error = prepare_sql("SELECT region, SUM(amount) OVER (PARTITION BY region) AS s FROM Sale GROUP BY region")
        .expect_err("a window next to GROUP BY must be rejected");
    assert!(format!("{error}").contains("cannot be combined"), "unexpected mix error: {error}");
}

#[test]
fn window_in_order_by_is_rejected() {
    assert!(prepare_sql("SELECT sale_id FROM Sale ORDER BY SUM(amount) OVER (PARTITION BY region)").is_err());
}

#[test]
fn window_inside_aggregate_argument_is_rejected() {
    // The nested `OVER` call reaches the lowering as the outer `SUM` argument, so
    // the window rule rejects it instead of letting the window be dropped.
    let error = prepare_sql("SELECT SUM(SUM(amount) OVER (PARTITION BY region)) AS s FROM Sale")
        .expect_err("a window inside an aggregate argument must be rejected");
    let message = format!("{error}");
    assert!(message.contains("`SUM`"), "unexpected nested window error: {message}");
    assert!(message.contains("top-level SELECT item"), "unexpected nested window error: {message}");
}

#[test]
fn window_unimplemented_functions_are_rejected() {
    for call in ["NTILE(2)", "LAG(amount)", "LEAD(amount)", "FIRST_VALUE(amount)", "LAST_VALUE(amount)"] {
        let sql = format!("SELECT {call} OVER (PARTITION BY region ORDER BY amount) AS w FROM Sale");
        let error = prepare_sql(&sql).expect_err("an unimplemented window function must be rejected");
        assert!(format!("{error}").contains("is not supported by YYDB"), "unexpected function error: {error}");
    }
}

#[test]
fn window_distinct_is_rejected() {
    let error = prepare_sql("SELECT COUNT(DISTINCT amount) OVER (PARTITION BY region) AS c FROM Sale")
        .expect_err("DISTINCT inside a window call must be rejected");
    assert!(format!("{error}").contains("remove DISTINCT"), "unexpected distinct error: {error}");
}

#[test]
fn window_ranking_arity_is_checked() {
    let error = prepare_sql("SELECT ROW_NUMBER(1) OVER (PARTITION BY region ORDER BY amount) AS rn FROM Sale")
        .expect_err("ROW_NUMBER takes no argument");
    assert!(format!("{error}").contains("takes no arguments"), "unexpected arity error: {error}");

    let error = prepare_sql("SELECT SUM() OVER (PARTITION BY region) AS s FROM Sale").expect_err("SUM requires one argument");
    assert!(format!("{error}").contains("requires exactly one argument"), "unexpected arity error: {error}");
}
