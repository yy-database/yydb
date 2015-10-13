//! CLI entry of the SQLite-referenced measurement harness.
//!
//! `yydb-bench [--rows N] [--point-ops N] [--range-scans N] [--range-len N]
//! [--batch-size N] [--single-inserts N] [--value-bytes N] [--dir PATH]
//! [--baseline-out PATH]`

use std::{io::Write, path::PathBuf};

use yydb_bench::{Baseline, BenchConfig, run};

fn main() {
    let mut config = BenchConfig::default();
    let mut dir = std::env::temp_dir().join("yydb-bench");
    let mut baseline_out: Option<PathBuf> = None;

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
            other => panic!("unknown argument `{other}`"),
        }
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
