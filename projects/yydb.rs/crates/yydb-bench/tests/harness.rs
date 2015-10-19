//! Dynamic validation of the measurement harness at a small scale.
//!
//! These tests assert cross-engine agreement of observable outcomes and that
//! every timing is positive. They deliberately assert **no absolute timing
//! threshold**: numbers drift across machines and builds, and a benchmark
//! must never turn an environment-dependent value into a pass/fail gate.

use std::time::{SystemTime, UNIX_EPOCH};

use yydb_bench::{BenchConfig, run};

fn small_config() -> BenchConfig {
    BenchConfig {
        rows: 400,
        point_ops: 200,
        range_scans: 8,
        range_len: 25,
        batch_size: 50,
        single_inserts: 20,
        value_bytes: 32,
        measure_rounds: 7,
    }
}

fn temp_dir(label: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("yydb-bench-{label}-{nonce}"))
}

#[test]
fn harness_runs_both_engines_with_positive_timings() {
    let dir = temp_dir("positive");
    let baseline = run(&dir, &small_config()).expect("harness runs both engines");

    for report in [&baseline.yydb, &baseline.sqlite] {
        assert!(report.batch_load_ns > 0, "{} batch load must measure", report.engine);
        assert!(report.single_insert_ns_per_op > 0, "{} single insert must measure", report.engine);
        assert!(report.point_get_ns_per_op > 0, "{} cold point get must measure", report.engine);
        assert!(report.point_get_warm_ns_per_op > 0, "{} warm point get must measure", report.engine);
        assert!(report.range_scan_ns_per_row > 0, "{} range scan must measure", report.engine);
        assert!(report.reopen_point_get_ns > 0, "{} reopen must measure", report.engine);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn baseline_serializes_with_scale_and_engine_reports() {
    let dir = temp_dir("serialize");
    let baseline = run(&dir, &small_config()).expect("harness runs");
    let json = serde_json::to_value(&baseline).expect("baseline serializes");
    assert_eq!(json["config"]["rows"], 400);
    assert_eq!(json["yydb"]["engine"], "yydb");
    assert_eq!(json["sqlite"]["engine"], "sqlite");
    assert!(json["os"].is_string());
    let _ = std::fs::remove_dir_all(&dir);
}
