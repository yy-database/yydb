//! CLI entry of the SQLite-referenced measurement harness.
//!
//! `yydb-bench [--rows N] [--point-ops N] [--range-scans N] [--range-len N]
//! [--batch-size N] [--single-inserts N] [--value-bytes N] [--dir PATH]
//! [--baseline-out PATH]`

use std::{io::Write, path::PathBuf};

use yydb_bench::{Baseline, BenchConfig, run, profile};

fn main() {
    let mut config = BenchConfig::default();
    let mut dir = std::env::temp_dir().join("yydb-bench");
    let mut baseline_out: Option<PathBuf> = None;
    let mut profile_only = false;

    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || {
            args.next()
                .and_then(|text| text.parse::<usize>().ok())
                .unwrap_or_else(|| panic!("{flag} expects a positive integer"))
        };
        match flag.as_str() {
            "--rows" => config.rows = value(),
            "--point-ops" => config.point_ops = value(),
            "--range-scans" => config.range_scans = value(),
            "--range-len" => config.range_len = value(),
            "--batch-size" => config.batch_size = value(),
            "--single-inserts" => config.single_inserts = value(),
            "--value-bytes" => config.value_bytes = value(),
            "--dir" => dir = PathBuf::from(args.next().unwrap_or_else(|| panic!("--dir expects a path"))),
            "--baseline-out" => {
                baseline_out = Some(PathBuf::from(args.next().unwrap_or_else(|| panic!("--baseline-out expects a path"))))
            }
            "--profile" => profile_only = true,
            other => panic!("unknown argument `{other}`"),
        }
    }

    if profile_only {
        let report = profile(&dir, &config).unwrap_or_else(|error| {
            eprintln!("profile failed: {error}");
            std::process::exit(1);
        });
        print_profile(&report);
        return;
    }

    let baseline = run(&dir, &config).unwrap_or_else(|error| {
        eprintln!("bench failed: {error}");
        std::process::exit(1);
    });
    print_report(&baseline);
    if let Some(path) = baseline_out {
        let json = serde_json::to_string_pretty(&baseline).expect("baseline serializes");
        let mut file = std::fs::File::create(&path).unwrap_or_else(|error| panic!("cannot write {}: {error}", path.display()));
        file.write_all(json.as_bytes()).expect("write baseline");
        println!("baseline written to {}", path.display());
    }
}

fn print_report(baseline: &Baseline) {
    let config = &baseline.config;
    println!(
        "yydb-bench · os={} · rows={} · point-ops={} · range={}x{} · batch={} · single={} · value={}B",
        baseline.os,
        config.rows,
        config.point_ops,
        config.range_scans,
        config.range_len,
        config.batch_size,
        config.single_inserts,
        config.value_bytes
    );
    println!("{:<24} {:>14} {:>14} {:>10}", "workload", "yydb", "sqlite", "yydb/sqlite");
    row("batch load (ns)", baseline.yydb.batch_load_ns, baseline.sqlite.batch_load_ns);
    row("single insert (ns/op)", baseline.yydb.single_insert_ns_per_op, baseline.sqlite.single_insert_ns_per_op);
    row("point get cold (ns/op)", baseline.yydb.point_get_ns_per_op, baseline.sqlite.point_get_ns_per_op);
    row("point get warm (ns/op)", baseline.yydb.point_get_warm_ns_per_op, baseline.sqlite.point_get_warm_ns_per_op);
    row("range scan (ns/row)", baseline.yydb.range_scan_ns_per_row, baseline.sqlite.range_scan_ns_per_row);
    row("reopen open (ns)", baseline.yydb.reopen_open_ns, baseline.sqlite.reopen_open_ns);
    row("reopen+100 gets (ns)", baseline.yydb.reopen_point_get_ns, baseline.sqlite.reopen_point_get_ns);
    row("empty reopen (ns)", baseline.yydb.empty_reopen_ns, baseline.sqlite.empty_reopen_ns);
    if let Some(publish) = &baseline.yydb.publish {
        println!();
        println!(
            "yydb publication · commits={} · frames={} · page-images={} · fsyncs={}",
            publish.commits, publish.frames, publish.page_images, publish.fsyncs
        );
        println!(
            "  batch commit share   {:>10.1}%  ({} of {})",
            publish.batch_commit_ns as f64 / baseline.yydb.batch_load_ns.max(1) as f64 * 100.0,
            publish.batch_commit_ns,
            baseline.yydb.batch_load_ns
        );
        let batch = &publish.batch;
        let batch_commits = batch.commits.max(1);
        println!(
            "  batch commits={} · frames={} · page-images={} · bytes={} · fsyncs={}",
            batch.commits, batch.frames, batch.page_images, batch.appended_bytes, batch.fsyncs
        );
        println!(
            "    per commit: frames={} page-images={} bytes={}",
            batch.frames / batch_commits,
            batch.page_images / batch_commits,
            batch.appended_bytes / batch_commits
        );
        println!(
            "    commit attribution: encode {:>10} ns · append {:>10} ns · fsync {:>10} ns · shm {:>10} ns · residual {:>10} ns",
            batch.encode_ns,
            batch.append_ns,
            batch.sync_ns,
            batch.shm_ns,
            publish.batch_commit_ns.saturating_sub(batch.encode_ns + batch.append_ns + batch.sync_ns + batch.shm_ns)
        );
        println!(
            "  per autocommit insert: frames={} fsyncs={}",
            publish.single_insert_frames_per_op, publish.single_insert_fsyncs_per_op
        );
        println!(
            "    encode {:>8} ns · append {:>8} ns · fsync {:>8} ns · shm {:>8} ns",
            publish.single_insert_encode_ns_per_op,
            publish.single_insert_append_ns_per_op,
            publish.single_insert_sync_ns_per_op,
            publish.single_insert_shm_ns_per_op
        );
        let op = baseline.yydb.single_insert_ns_per_op;
        let attributed = publish.single_insert_encode_ns_per_op
            + publish.single_insert_append_ns_per_op
            + publish.single_insert_sync_ns_per_op
            + publish.single_insert_shm_ns_per_op;
        println!("    op total {:>8} ns · non-fsync CPU {:>8} ns", op, op.saturating_sub(publish.single_insert_sync_ns_per_op));
        println!("    attributed {:>8} ns of {}", attributed, op);
        println!("  main file {} B · wal {} B", publish.main_file_bytes, publish.wal_file_bytes);
    }
    println!("Raw numbers only: no absolute SLO is asserted. Compare on the same machine.");
}

fn row(label: &str, yydb: u128, sqlite: u128) {
    let ratio = yydb as f64 / sqlite.max(1) as f64;
    println!("{label:<24} {yydb:>14} {sqlite:>14} {ratio:>9.2}x");
}

fn print_profile(report: &yydb_bench::ProfileReport) {
    println!(
        "yydb profile · rows={} · range-len={} · calls={}",
        report.rows, report.range_len, report.calls
    );
    println!(
        "{:<20} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10} {:>6}",
        "shape (µs/call)", "total", "bind", "metadata", "drift", "plan", "window", "exec", "rows"
    );
    stage("empty window", &report.empty_window);
    stage("one row", &report.one_row);
    stage("range window", &report.range_window);
    stage("window no sort", &report.window_no_sort);
    stage("point get", &report.point_get);
    println!(
        "per-row: window {:>7.1} ns · exec {:>7.1} ns",
        per_row(report.range_window.window_read_ns, report.one_row.window_read_ns, report.range_window.rows),
        per_row(report.range_window.exec_ns, report.one_row.exec_ns, report.range_window.rows),
    );
    println!(
        "{:<20} {:>10} {:>10} {:>10} {:>10} {:>10}",
        "exec split (µs/call)", "exec", "scan", "filter", "project", "sort"
    );
    exec_split("range window", &report.range_window);
    exec_split("one row", &report.one_row);
    exec_split("point get", &report.point_get);
    println!(
        "prepare_sql {:>8} ns/call · unprepared point {:>8} ns/call",
        report.prepare_ns, report.unprepared_point_ns
    );
    println!(
        "insert in txn {:>8} ns/row · insert prepare {:>8} ns · commit {:>8} ns",
        report.insert_row_ns, report.insert_prepare_ns, report.commit_per_batch_ns
    );
    println!("Raw numbers only: timers are taken on the profiling path alone.");
}

fn per_row(range_ns: u128, baseline_ns: u128, rows: usize) -> f64 {
    let extra_rows = rows.saturating_sub(1);
    if extra_rows == 0 {
        return 0.0;
    }
    range_ns.saturating_sub(baseline_ns) as f64 / extra_rows as f64
}

fn exec_split(label: &str, report: &yydb_bench::StageReport) {
    println!(
        "{:<20} {:>10.2} {:>10.2} {:>10.2} {:>10.2} {:>10.2}",
        label,
        report.exec_ns as f64 / 1000.0,
        report.scan_ns as f64 / 1000.0,
        report.filter_ns as f64 / 1000.0,
        report.project_ns as f64 / 1000.0,
        report.sort_ns as f64 / 1000.0,
    );
}

fn stage(label: &str, report: &yydb_bench::StageReport) {
    println!(
        "{:<20} {:>10.2} {:>10.2} {:>10.2} {:>10.2} {:>10.2} {:>10.2} {:>10.2} {:>6}",
        label,
        report.total_ns as f64 / 1000.0,
        report.bind_ns as f64 / 1000.0,
        report.metadata_ns as f64 / 1000.0,
        report.drift_ns as f64 / 1000.0,
        report.plan_ns as f64 / 1000.0,
        report.window_read_ns as f64 / 1000.0,
        report.exec_ns as f64 / 1000.0,
        report.rows
    );
}
