//! SQLite-referenced measurement harness for YYDB.
//!
//! Runs the same dataset and equivalent workloads against YYDB and SQLite on
//! the same machine: transactional batch load, autocommit single inserts,
//! point gets (cold and warm), key-range scans, and close/reopen followed by
//! point gets. Every report carries the method, scale and raw numbers; the
//! harness deliberately asserts **no absolute SLO** because timings drift
//! across machines and builds. Compare engines within one run, or compare
//! runs on the same machine.

use std::{path::Path, time::Instant};

use serde::Serialize;
use yydb::Value;

/// Workload scale for one measurement run.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct BenchConfig {
    /// Rows loaded transactionally before the read workloads.
    pub rows: usize,
    /// Point gets per cold/warm pass.
    pub point_ops: usize,
    /// Number of key-range scans.
    pub range_scans: usize,
    /// Rows per range scan.
    pub range_len: usize,
    /// Rows per transaction during the batch load.
    pub batch_size: usize,
    /// Autocommit single inserts measured after the load.
    pub single_inserts: usize,
    /// Payload size in bytes.
    pub value_bytes: usize,
    /// Recorded rounds in the alternating same-process measurement.
    pub measure_rounds: usize,
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            rows: 20_000,
            point_ops: 10_000,
            range_scans: 200,
            range_len: 100,
            batch_size: 1_000,
            single_inserts: 200,
            value_bytes: 64,
            measure_rounds: 7,
        }
    }
}

/// Raw timings of one engine, all in nanoseconds.
#[derive(Debug, Clone, Serialize)]
pub struct EngineReport {
    /// Engine label (`yydb` or `sqlite`).
    pub engine: &'static str,
    /// Transactional load of the full dataset.
    pub batch_load_ns: u128,
    /// One autocommit insert, averaged.
    pub single_insert_ns_per_op: u128,
    /// First (cold) point-get pass, averaged.
    pub point_get_ns_per_op: u128,
    /// Second (warm) point-get pass, averaged.
    pub point_get_warm_ns_per_op: u128,
    /// Range scan, averaged per returned row.
    pub range_scan_ns_per_row: u128,
    /// Open a populated database file, including any recovery work.
    pub reopen_open_ns: u128,
    /// 100 point gets on the reopened handle, excluding the open.
    pub reopen_point_get_ns: u128,
    /// Open and close a database file that holds no schema and no rows.
    pub empty_reopen_ns: u128,
    /// Publication attribution; only YYDB writes through a WAL sidecar report it.
    pub publish: Option<PublishReport>,
}

/// Publication volume and time for the batch load's `commit()` calls alone.
#[derive(Debug, Clone, Serialize)]
pub struct BatchPublishReport {
    /// Commits issued by the batch loop.
    pub commits: u64,
    /// Frames appended by those commits.
    pub frames: u64,
    /// Page-image frames appended by those commits.
    pub page_images: u64,
    /// WAL bytes appended by those commits.
    pub appended_bytes: u64,
    /// Frame encoding time for those commits.
    pub encode_ns: u128,
    /// WAL append time for those commits.
    pub append_ns: u128,
    /// Durability barrier time for those commits.
    pub sync_ns: u128,
    /// Durability barriers issued by those commits.
    pub fsyncs: u64,
    /// `-shm` mirror writes caused by those commits.
    pub shm_writes: u64,
    /// `-shm` mirror time caused by those commits.
    pub shm_ns: u128,
}

/// Attribution of YYDB write time between frame encoding, WAL append, the
/// durability barrier and the `-shm` mirror.
///
/// `commits`, `frames`, `page_images`, `encode_ns`, `append_ns`, `sync_ns`,
/// `fsyncs`, `shm_writes` and `shm_ns` are cumulative for the whole run.
/// `single_insert_*_per_op` are the deltas across the autocommit insert loop.
#[derive(Debug, Clone, Serialize)]
pub struct PublishReport {
    /// Committed WAL transactions across the run.
    pub commits: u64,
    /// Frames appended across those transactions.
    pub frames: u64,
    /// Page-image frames appended across those transactions.
    pub page_images: u64,
    /// WAL bytes appended across those transactions.
    pub appended_bytes: u64,
    /// Time encoding frames and the commit digest.
    pub encode_ns: u128,
    /// Time inside the WAL append call.
    pub append_ns: u128,
    /// Time inside the durability barrier.
    pub sync_ns: u128,
    /// Durability barrier calls.
    pub fsyncs: u64,
    /// `-shm` mirror writes.
    pub shm_writes: u64,
    /// Time spent writing the `-shm` mirror.
    pub shm_ns: u128,
    /// Publication attribution of the batch load's commits only.
    pub batch: BatchPublishReport,
    /// Time spent inside `commit()` during the batch load.
    pub batch_commit_ns: u128,
    /// Number of `commit()` calls during the batch load.
    pub batch_commits: u64,
    /// WAL frames per autocommit insert.
    pub single_insert_frames_per_op: u128,
    /// Durability barriers per autocommit insert.
    pub single_insert_fsyncs_per_op: u128,
    /// Frame encoding time per autocommit insert.
    pub single_insert_encode_ns_per_op: u128,
    /// WAL append time per autocommit insert.
    pub single_insert_append_ns_per_op: u128,
    /// Durability barrier time per autocommit insert.
    pub single_insert_sync_ns_per_op: u128,
    /// `-shm` mirror time per autocommit insert.
    pub single_insert_shm_ns_per_op: u128,
    /// Main `YDPG` file length after the batch load and single inserts.
    pub main_file_bytes: u64,
    /// WAL sidecar length at the same point.
    pub wal_file_bytes: u64,
}

/// One recorded baseline: scale, environment and both engine reports.
#[derive(Debug, Serialize)]
pub struct Baseline {
    /// Workload scale used for both engines.
    pub config: BenchConfig,
    /// Operating system the run happened on.
    pub os: &'static str,
    /// YYDB timings.
    pub yydb: EngineReport,
    /// SQLite (bundled rusqlite) timings.
    pub sqlite: EngineReport,
}

/// Deterministic key of row `index`; lexicographic order equals numeric order.
pub fn key_at(index: usize) -> String {
    format!("key-{index:010}")
}

/// Deterministic payload of row `index`.
pub fn value_at(index: usize, len: usize) -> Vec<u8> {
    (0..len).map(|offset| ((index.wrapping_mul(31) + offset) % 251) as u8).collect()
}

/// Runs both engines under `dir` and returns the recorded baseline.
pub fn run(dir: &Path, config: &BenchConfig) -> Result<Baseline, String> {
    std::fs::create_dir_all(dir).map_err(|error| format!("cannot create bench dir: {error}"))?;
    let yydb = run_yydb(dir, config)?;
    let sqlite = run_sqlite(dir, config)?;
    Ok(Baseline { config: *config, os: std::env::consts::OS, yydb, sqlite })
}

fn run_yydb(dir: &Path, cfg: &BenchConfig) -> Result<EngineReport, String> {
    let path = dir.join("bench.yydb");
    // Remove the primary file *and* its WAL/SHM companions: replaying a stale
    // journal into a fresh primary silently resurrects the previous dataset.
    for suffix in ["", "-wal", "-shm"] {
        let companion = dir.join(format!("bench.yydb{suffix}"));
        let _ = std::fs::remove_file(&companion);
    }
    let conn = yydb::Connection::open_with_flags(&path, yydb::OpenFlags::wal()).map_err(|e| format!("yydb open: {e}"))?;
    conn.ensure_schema("table Bench { @@id: utf8, payload: bytes }").map_err(|e| format!("yydb schema: {e}"))?;

    let empty_path = dir.join("empty.yydb");
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(dir.join(format!("empty.yydb{suffix}")));
    }
    let created =
        yydb::Connection::open_with_flags(&empty_path, yydb::OpenFlags::wal()).map_err(|e| format!("yydb empty open: {e}"))?;
    created.close();
    drop(created);
    let empty_started = Instant::now();
    let reopened_empty = yydb::Connection::open_with_flags(&empty_path, yydb::OpenFlags::wal())
        .map_err(|e| format!("yydb empty reopen: {e}"))?;
    reopened_empty.close();
    let empty_reopen_ns = empty_started.elapsed().as_nanos();
    drop(reopened_empty);

    let before_batch = publish_stats(&conn);
    let started = Instant::now();
    let mut batch_commit_ns = 0_u128;
    let mut batch_commits = 0_u64;
    for chunk_start in (0..cfg.rows).step_by(cfg.batch_size.max(1)) {
        conn.begin().map_err(|e| format!("yydb begin: {e}"))?;
        for index in chunk_start..(chunk_start + cfg.batch_size.max(1)).min(cfg.rows) {
            yydb_insert(&conn, index, cfg)?;
        }
        let commit_started = Instant::now();
        conn.commit().map_err(|e| format!("yydb commit: {e}"))?;
        batch_commit_ns += commit_started.elapsed().as_nanos();
        batch_commits += 1;
    }
    let batch_load_ns = started.elapsed().as_nanos();
    let batch_delta = stats_delta(&before_batch, &publish_stats(&conn));
    verify_yydb_count(&conn, cfg.rows, "yydb post-load count")?;

    let before_single = publish_stats(&conn);
    let started = Instant::now();
    for index in 0..cfg.single_inserts {
        yydb_insert(&conn, cfg.rows + index, cfg)?;
    }
    let single_insert_ns_per_op = started.elapsed().as_nanos() / cfg.single_inserts.max(1) as u128;
    let after_single = publish_stats(&conn);
    let single_delta = stats_delta(&before_single, &after_single);
    let single_ops = cfg.single_inserts.max(1) as u128;

    let point_get_ns_per_op = yydb_point_pass(&conn, cfg, "yydb cold point get")?;
    let point_get_warm_ns_per_op = yydb_point_pass(&conn, cfg, "yydb warm point get")?;
    let range_scan_ns_per_row = yydb_range_pass(&conn, cfg)?;

    let main_file_bytes = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
    let wal_file_bytes = std::fs::metadata(dir.join("bench.yydb-wal")).map(|meta| meta.len()).unwrap_or(0);

    conn.close();
    // The writer lock is released when the handle is dropped, not on `close`.
    drop(conn);
    let started = Instant::now();
    let reopened = yydb::Connection::open_with_flags(&path, yydb::OpenFlags::wal()).map_err(|e| format!("yydb reopen: {e}"))?;
    let reopen_open_ns = started.elapsed().as_nanos();
    let get_started = Instant::now();
    for op in 0..100usize {
        let index = (op * 7919) % cfg.rows.max(1);
        let rows = reopened
            .query_sql_with_params("SELECT payload FROM Bench WHERE id = ?1", &[Value::Text(key_at(index))])
            .map_err(|e| format!("yydb reopen point get: {e}"))?;
        if rows.len() != 1 {
            return Err(format!("yydb reopen point get returned {} rows", rows.len()));
        }
    }
    let reopen_point_get_ns = get_started.elapsed().as_nanos();
    reopened.close();

    let publish = PublishReport {
        commits: after_single.wal.commits,
        frames: after_single.wal.frames,
        page_images: after_single.wal.page_images,
        appended_bytes: after_single.wal.appended_bytes,
        encode_ns: after_single.wal.encode_ns,
        append_ns: after_single.wal.append_ns,
        sync_ns: after_single.wal.sync_ns,
        fsyncs: after_single.wal.fsyncs,
        shm_writes: after_single.shm_writes,
        shm_ns: after_single.shm_ns,
        batch: BatchPublishReport {
            commits: batch_delta.wal.commits,
            frames: batch_delta.wal.frames,
            page_images: batch_delta.wal.page_images,
            appended_bytes: batch_delta.wal.appended_bytes,
            encode_ns: batch_delta.wal.encode_ns,
            append_ns: batch_delta.wal.append_ns,
            sync_ns: batch_delta.wal.sync_ns,
            fsyncs: batch_delta.wal.fsyncs,
            shm_writes: batch_delta.shm_writes,
            shm_ns: batch_delta.shm_ns,
        },
        batch_commit_ns,
        batch_commits,
        single_insert_frames_per_op: single_delta.wal.frames as u128 / single_ops,
        single_insert_fsyncs_per_op: single_delta.wal.fsyncs as u128 / single_ops,
        single_insert_encode_ns_per_op: single_delta.wal.encode_ns / single_ops,
        single_insert_append_ns_per_op: single_delta.wal.append_ns / single_ops,
        single_insert_sync_ns_per_op: single_delta.wal.sync_ns / single_ops,
        single_insert_shm_ns_per_op: single_delta.shm_ns / single_ops,
        main_file_bytes,
        wal_file_bytes,
    };

    Ok(EngineReport {
        engine: "yydb",
        batch_load_ns,
        single_insert_ns_per_op,
        point_get_ns_per_op,
        point_get_warm_ns_per_op,
        range_scan_ns_per_row,
        reopen_open_ns,
        reopen_point_get_ns,
        empty_reopen_ns,
        publish: Some(publish),
    })
}

fn publish_stats(conn: &yydb::Connection) -> yydb_storage::PublishStats {
    conn.publish_stats().unwrap_or_default()
}

fn stats_delta(before: &yydb_storage::PublishStats, after: &yydb_storage::PublishStats) -> yydb_storage::PublishStats {
    yydb_storage::PublishStats {
        wal: yydb_storage::WalWriteStats {
            commits: after.wal.commits - before.wal.commits,
            frames: after.wal.frames - before.wal.frames,
            page_images: after.wal.page_images - before.wal.page_images,
            appended_bytes: after.wal.appended_bytes - before.wal.appended_bytes,
            encode_ns: after.wal.encode_ns - before.wal.encode_ns,
            append_ns: after.wal.append_ns - before.wal.append_ns,
            sync_ns: after.wal.sync_ns - before.wal.sync_ns,
            fsyncs: after.wal.fsyncs - before.wal.fsyncs,
        },
        shm_writes: after.shm_writes - before.shm_writes,
        shm_ns: after.shm_ns - before.shm_ns,
    }
}

fn verify_yydb_count(conn: &yydb::Connection, expected: usize, context: &str) -> Result<(), String> {
    let rows = conn.query_sql("SELECT COUNT(*) AS n FROM Bench").map_err(|e| format!("{context}: {e}"))?;
    match rows.first().and_then(|row| row.get("n")) {
        Some(Value::I64(count)) if *count as usize == expected => Ok(()),
        other => Err(format!("{context} is {other:?}, expected {expected} rows")),
    }
}

fn yydb_insert(conn: &yydb::Connection, index: usize, cfg: &BenchConfig) -> Result<(), String> {
    let values = [Value::Text(key_at(index)), Value::Bytes(value_at(index, cfg.value_bytes))];
    conn.execute_sql_write("INSERT INTO Bench (id, payload) VALUES (?1, ?2)", &values)
        .map_err(|e| format!("yydb insert `{}`: {e}", key_at(index)))?;
    Ok(())
}

fn yydb_point_pass(conn: &yydb::Connection, cfg: &BenchConfig, context: &str) -> Result<u128, String> {
    let statement =
        conn.prepare_sql("SELECT payload FROM Bench WHERE id = ?1").map_err(|e| format!("{context} prepare: {e}"))?;
    let started = Instant::now();
    for op in 0..cfg.point_ops {
        let index = (op * 7919) % cfg.rows.max(1);
        let rows =
            conn.query_prepared_ordered(&statement, &[Value::Text(key_at(index))]).map_err(|e| format!("{context}: {e}"))?;
        if rows.len() != 1 {
            return Err(format!("{context} returned {} rows for `{}`", rows.len(), key_at(index)));
        }
    }
    Ok(started.elapsed().as_nanos() / cfg.point_ops.max(1) as u128)
}

fn yydb_range_pass(conn: &yydb::Connection, cfg: &BenchConfig) -> Result<u128, String> {
    let statement = conn
        .prepare_sql("SELECT id, payload FROM Bench WHERE id >= ?1 AND id < ?2 ORDER BY id")
        .map_err(|e| format!("yydb range prepare: {e}"))?;
    let mut scanned = 0usize;
    let started = Instant::now();
    for scan in 0..cfg.range_scans {
        let start_index = (scan * 104_729) % cfg.rows.max(1);
        let end_index = (start_index + cfg.range_len).min(cfg.rows);
        let from = Value::Text(key_at(start_index));
        let to = Value::Text(key_at(end_index));
        let rows = conn.query_prepared_ordered(&statement, &[from, to]).map_err(|e| format!("yydb range scan: {e}"))?;
        if rows.len() != end_index - start_index {
            return Err(format!("yydb range scan returned {} rows, expected {}", rows.len(), end_index - start_index));
        }
        scanned += rows.len();
    }
    Ok(started.elapsed().as_nanos() / scanned.max(1) as u128)
}

fn run_sqlite(dir: &Path, cfg: &BenchConfig) -> Result<EngineReport, String> {
    let path = dir.join("bench.sqlite");
    let _ = std::fs::remove_file(&path);
    let conn = rusqlite::Connection::open(&path).map_err(|e| format!("sqlite open: {e}"))?;
    conn.execute_batch("PRAGMA journal_mode = WAL; CREATE TABLE Bench (id TEXT PRIMARY KEY, payload BLOB NOT NULL)")
        .map_err(|e| format!("sqlite schema: {e}"))?;

    let empty_path = dir.join("empty.sqlite");
    let _ = std::fs::remove_file(&empty_path);
    let created = rusqlite::Connection::open(&empty_path).map_err(|e| format!("sqlite empty open: {e}"))?;
    drop(created);
    let empty_started = Instant::now();
    let reopened_empty = rusqlite::Connection::open(&empty_path).map_err(|e| format!("sqlite empty reopen: {e}"))?;
    let empty_reopen_ns = empty_started.elapsed().as_nanos();
    drop(reopened_empty);

    let started = Instant::now();
    for chunk_start in (0..cfg.rows).step_by(cfg.batch_size.max(1)) {
        conn.execute_batch("BEGIN").map_err(|e| format!("sqlite begin: {e}"))?;
        for index in chunk_start..(chunk_start + cfg.batch_size.max(1)).min(cfg.rows) {
            sqlite_insert(&conn, index, cfg)?;
        }
        conn.execute_batch("COMMIT").map_err(|e| format!("sqlite commit: {e}"))?;
    }
    let batch_load_ns = started.elapsed().as_nanos();
    let count: usize = conn
        .query_row("SELECT COUNT(*) FROM Bench", [], |row| row.get::<_, usize>(0))
        .map_err(|e| format!("sqlite post-load count: {e}"))?;
    if count != cfg.rows {
        return Err(format!("sqlite post-load count is {count}, expected {}", cfg.rows));
    }

    let started = Instant::now();
    for index in 0..cfg.single_inserts {
        sqlite_insert(&conn, cfg.rows + index, cfg)?;
    }
    let single_insert_ns_per_op = started.elapsed().as_nanos() / cfg.single_inserts.max(1) as u128;

    let point_get_ns_per_op = sqlite_point_pass(&conn, cfg, "sqlite cold point get")?;
    let point_get_warm_ns_per_op = sqlite_point_pass(&conn, cfg, "sqlite warm point get")?;
    let range_scan_ns_per_row = sqlite_range_pass(&conn, cfg)?;

    drop(conn);
    let started = Instant::now();
    let reopened = rusqlite::Connection::open(&path).map_err(|e| format!("sqlite reopen: {e}"))?;
    let reopen_open_ns = started.elapsed().as_nanos();
    let get_started = Instant::now();
    for op in 0..100usize {
        let index = (op * 7919) % cfg.rows.max(1);
        let mut statement =
            reopened.prepare("SELECT payload FROM Bench WHERE id = ?1").map_err(|e| format!("sqlite reopen prepare: {e}"))?;
        let (count, _bytes) = read_payload_rows(&mut statement, rusqlite::params![key_at(index)], "sqlite reopen point get")?;
        if count != 1 {
            return Err(format!("sqlite reopen point get returned {count} rows"));
        }
    }
    let reopen_point_get_ns = get_started.elapsed().as_nanos();
    drop(reopened);

    Ok(EngineReport {
        engine: "sqlite",
        batch_load_ns,
        single_insert_ns_per_op,
        point_get_ns_per_op,
        point_get_warm_ns_per_op,
        range_scan_ns_per_row,
        reopen_open_ns,
        reopen_point_get_ns,
        empty_reopen_ns,
        publish: None,
    })
}

fn sqlite_insert(conn: &rusqlite::Connection, index: usize, cfg: &BenchConfig) -> Result<(), String> {
    conn.execute(
        "INSERT INTO Bench (id, payload) VALUES (?1, ?2)",
        rusqlite::params![key_at(index), value_at(index, cfg.value_bytes)],
    )
    .map_err(|e| format!("sqlite insert `{}`: {e}", key_at(index)))?;
    Ok(())
}

fn sqlite_point_pass(conn: &rusqlite::Connection, cfg: &BenchConfig, context: &str) -> Result<u128, String> {
    let mut statement =
        conn.prepare("SELECT payload FROM Bench WHERE id = ?1").map_err(|e| format!("{context} prepare: {e}"))?;
    let started = Instant::now();
    for op in 0..cfg.point_ops {
        let index = (op * 7919) % cfg.rows.max(1);
        let (count, _bytes) = read_payload_rows(&mut statement, rusqlite::params![key_at(index)], context)?;
        if count != 1 {
            return Err(format!("{context} returned {count} rows for `{}`", key_at(index)));
        }
    }
    Ok(started.elapsed().as_nanos() / cfg.point_ops.max(1) as u128)
}

/// Reads a `payload`-only result set into owned values.
///
/// The yydb side materialises every selected column, so the SQLite side reads
/// the value as well instead of advancing the cursor without touching it.
/// Returns `(rows, payload bytes)` so the caller can keep the result alive.
fn read_payload_rows(
    statement: &mut rusqlite::Statement<'_>,
    params: impl rusqlite::Params,
    context: &str,
) -> Result<(usize, usize), String> {
    let mut rows = statement.query(params).map_err(|e| format!("{context}: {e}"))?;
    let mut count = 0usize;
    let mut bytes = 0usize;
    while let Some(row) = rows.next().map_err(|e| format!("{context}: {e}"))? {
        let payload: Vec<u8> = row.get(0).map_err(|e| format!("{context}: {e}"))?;
        bytes += payload.len();
        count += 1;
    }
    Ok((count, bytes))
}

/// Reads a `(id, payload)` result set into owned values, matching the yydb
/// materialisation of the same projection.
fn read_keyed_rows(
    statement: &mut rusqlite::Statement<'_>,
    params: impl rusqlite::Params,
    context: &str,
) -> Result<(usize, usize), String> {
    let mut rows = statement.query(params).map_err(|e| format!("{context}: {e}"))?;
    let mut count = 0usize;
    let mut bytes = 0usize;
    while let Some(row) = rows.next().map_err(|e| format!("{context}: {e}"))? {
        let id: String = row.get(0).map_err(|e| format!("{context}: {e}"))?;
        let payload: Vec<u8> = row.get(1).map_err(|e| format!("{context}: {e}"))?;
        bytes += id.len() + payload.len();
        count += 1;
    }
    Ok((count, bytes))
}

fn sqlite_range_pass(conn: &rusqlite::Connection, cfg: &BenchConfig) -> Result<u128, String> {
    let mut statement = conn
        .prepare("SELECT id, payload FROM Bench WHERE id >= ?1 AND id < ?2 ORDER BY id")
        .map_err(|e| format!("sqlite range prepare: {e}"))?;
    let mut scanned = 0usize;
    let started = Instant::now();
    for scan in 0..cfg.range_scans {
        let start_index = (scan * 104_729) % cfg.rows.max(1);
        let end_index = (start_index + cfg.range_len).min(cfg.rows);
        let (count, _bytes) = read_keyed_rows(
            &mut statement,
            rusqlite::params![key_at(start_index), key_at(end_index)],
            "sqlite range scan",
        )?;
        if count != end_index - start_index {
            return Err(format!("sqlite range scan returned {count} rows, expected {}", end_index - start_index));
        }
        scanned += count;
    }
    Ok(started.elapsed().as_nanos() / scanned.max(1) as u128)
}

/// Stage timings of one read shape, averaged over its calls.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct StageReport {
    /// Parameter binding and validation.
    pub bind_ns: u128,
    /// Reserved metadata read plus schema and contract decode.
    pub metadata_ns: u128,
    /// Schema drift check, including the schema document fingerprint.
    pub drift_ns: u128,
    /// Record-key window planning.
    pub plan_ns: u128,
    /// Pager window read and its record decode.
    pub window_read_ns: u128,
    /// Plan execution: table decode, filters, projection, ordering.
    pub exec_ns: u128,
    /// Table scan inside `exec_ns`: record-map walk and row decode.
    pub scan_ns: u128,
    /// Row filtering inside `exec_ns`.
    pub filter_ns: u128,
    /// Projection rebuild inside `exec_ns`.
    pub project_ns: u128,
    /// Ordering inside `exec_ns`.
    pub sort_ns: u128,
    /// Other operators inside `exec_ns`.
    pub exec_other_ns: u128,
    /// Sum of the stages above.
    pub total_ns: u128,
    /// Rows emitted per call.
    pub rows: usize,
}

impl StageReport {
    fn averaged(profile: yydb::connection::ReadProfile, calls: usize) -> Self {
        let calls = calls.max(1);
        let divisor = calls as u128;
        Self {
            bind_ns: profile.bind_ns / divisor,
            metadata_ns: profile.metadata_ns / divisor,
            drift_ns: profile.drift_ns / divisor,
            plan_ns: profile.plan_ns / divisor,
            window_read_ns: profile.window_read_ns / divisor,
            exec_ns: profile.exec_ns / divisor,
            scan_ns: profile.scan_ns / divisor,
            filter_ns: profile.filter_ns / divisor,
            project_ns: profile.project_ns / divisor,
            sort_ns: profile.sort_ns / divisor,
            exec_other_ns: profile.exec_other_ns / divisor,
            total_ns: profile.total_ns() / divisor,
            rows: profile.scanned_rows / calls,
        }
    }
}

/// Fixed-cost decomposition of the YYDB read and write paths on the bench dataset.
#[derive(Debug, Clone, Serialize)]
pub struct ProfileReport {
    /// Dataset rows loaded before measuring.
    pub rows: usize,
    /// Rows per measured range window.
    pub range_len: usize,
    /// Calls behind each read shape.
    pub calls: usize,
    /// Range statement with an empty window: pure fixed cost.
    pub empty_window: StageReport,
    /// Same statement returning exactly one row.
    pub one_row: StageReport,
    /// Same statement returning `range_len` rows.
    pub range_window: StageReport,
    /// Same window without `ORDER BY`, to separate sort cost.
    pub window_no_sort: StageReport,
    /// Point statement returning one row.
    pub point_get: StageReport,
    /// One `prepare_sql` call, including its metadata basis read.
    pub prepare_ns: u128,
    /// One unprepared point read (`query_sql_with_params`).
    pub unprepared_point_ns: u128,
    /// One insert inside an open transaction, including its own preparation.
    pub insert_row_ns: u128,
    /// One insert statement preparation alone.
    pub insert_prepare_ns: u128,
    /// One batch commit of `batch_size` rows.
    pub commit_per_batch_ns: u128,
}

const RANGE_SQL: &str = "SELECT id, payload FROM Bench WHERE id >= ?1 AND id < ?2 ORDER BY id";
const UNSORTED_RANGE_SQL: &str = "SELECT id, payload FROM Bench WHERE id >= ?1 AND id < ?2";
const POINT_SQL: &str = "SELECT payload FROM Bench WHERE id = ?1";
const INSERT_SQL: &str = "INSERT INTO Bench (id, payload) VALUES (?1, ?2)";

/// Calls run before a shape is timed, so the first sample never absorbs cold
/// page reads or a freshly built binary.
const WARMUP_CALLS: usize = 20;

/// Runs one read shape in its own loop: `WARMUP_CALLS` untimed calls followed
/// by `calls` measured ones.
///
/// Shapes are timed separately because each walks a different record window,
/// so sharing a loop let the previous shape flush the pager cache and made
/// ordering an unnamed variable in the numbers.
fn measure_shape<F>(calls: usize, mut run: F) -> Result<yydb::connection::ReadProfile, String>
where
    F: FnMut(usize, &mut yydb::connection::ReadProfile) -> Result<(), String>,
{
    let mut warm = yydb::connection::ReadProfile::default();
    for scan in 0..WARMUP_CALLS.min(calls.max(1)) {
        run(scan, &mut warm)?;
    }
    let mut profile = yydb::connection::ReadProfile::default();
    for scan in 0..calls {
        run(scan, &mut profile)?;
    }
    Ok(profile)
}

/// Record keys of one measured range.
fn range_bounds(cfg: &BenchConfig, scan: usize) -> (usize, usize) {
    let start = (scan * 104_729) % cfg.rows.max(1);
    let end = (start + cfg.range_len).min(cfg.rows);
    (start, end)
}

/// Loads the bench dataset and reports where one read or write call spends its time.
pub fn profile(dir: &Path, cfg: &BenchConfig) -> Result<ProfileReport, String> {
    std::fs::create_dir_all(dir).map_err(|error| format!("cannot create profile dir: {error}"))?;
    let path = dir.join("profile.yydb");
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(dir.join(format!("profile.yydb{suffix}")));
    }
    let conn = yydb::Connection::open_with_flags(&path, yydb::OpenFlags::wal()).map_err(|e| format!("profile open: {e}"))?;
    conn.ensure_schema("table Bench { @@id: utf8, payload: bytes }").map_err(|e| format!("profile schema: {e}"))?;
    let load_started = Instant::now();
    for chunk_start in (0..cfg.rows).step_by(cfg.batch_size.max(1)) {
        conn.begin().map_err(|e| format!("profile begin: {e}"))?;
        for index in chunk_start..(chunk_start + cfg.batch_size.max(1)).min(cfg.rows) {
            yydb_insert(&conn, index, cfg)?;
        }
        conn.commit().map_err(|e| format!("profile commit: {e}"))?;
    }
    let _load_ns = load_started.elapsed().as_nanos();
    verify_yydb_count(&conn, cfg.rows, "profile post-load count")?;

    let calls = cfg.range_scans.max(1);
    let range_statement = conn.prepare_sql(RANGE_SQL).map_err(|e| format!("profile range prepare: {e}"))?;
    let unsorted_statement =
        conn.prepare_sql(UNSORTED_RANGE_SQL).map_err(|e| format!("profile unsorted prepare: {e}"))?;

    let empty = measure_shape(calls, |scan, profile| {
        let start_index = range_bounds(cfg, scan).0;
        let from = Value::Text(key_at(start_index));
        let to = Value::Text(key_at(start_index));
        let rows = conn
            .query_prepared_ordered_profiled(&range_statement, &[from, to], profile)
            .map_err(|e| format!("profile empty window: {e}"))?;
        if !rows.is_empty() {
            return Err(format!("profile empty window returned {} rows", rows.len()));
        }
        Ok(())
    })?;

    let single = measure_shape(calls, |scan, profile| {
        let start_index = range_bounds(cfg, scan).0;
        let from = Value::Text(key_at(start_index));
        let to = Value::Text(key_at((start_index + 1).min(cfg.rows)));
        let rows = conn
            .query_prepared_ordered_profiled(&range_statement, &[from, to], profile)
            .map_err(|e| format!("profile single row: {e}"))?;
        if rows.len() != 1 {
            return Err(format!("profile single row returned {} rows", rows.len()));
        }
        Ok(())
    })?;

    let window = measure_shape(calls, |scan, profile| {
        let (start_index, end_index) = range_bounds(cfg, scan);
        let from = Value::Text(key_at(start_index));
        let to = Value::Text(key_at(end_index));
        let rows = conn
            .query_prepared_ordered_profiled(&range_statement, &[from, to], profile)
            .map_err(|e| format!("profile range window: {e}"))?;
        if rows.len() != end_index - start_index {
            return Err(format!("profile range window returned {} rows", rows.len()));
        }
        Ok(())
    })?;

    let unsorted = measure_shape(calls, |scan, profile| {
        let (start_index, end_index) = range_bounds(cfg, scan);
        let from = Value::Text(key_at(start_index));
        let to = Value::Text(key_at(end_index));
        let rows = conn
            .query_prepared_ordered_profiled(&unsorted_statement, &[from, to], profile)
            .map_err(|e| format!("profile unsorted window: {e}"))?;
        if rows.len() != end_index - start_index {
            return Err(format!("profile unsorted window returned {} rows", rows.len()));
        }
        Ok(())
    })?;

    let point_statement = conn.prepare_sql(POINT_SQL).map_err(|e| format!("profile point prepare: {e}"))?;
    let point_calls = cfg.point_ops.max(1);
    let point = measure_shape(point_calls, |op, profile| {
        let index = (op * 7919) % cfg.rows.max(1);
        let rows = conn
            .query_prepared_ordered_profiled(&point_statement, &[Value::Text(key_at(index))], profile)
            .map_err(|e| format!("profile point get: {e}"))?;
        if rows.len() != 1 {
            return Err(format!("profile point get returned {} rows", rows.len()));
        }
        Ok(())
    })?;

    let prepare_calls = 100usize;
    let started = Instant::now();
    for _ in 0..prepare_calls {
        conn.prepare_sql(RANGE_SQL).map_err(|e| format!("profile prepare: {e}"))?;
    }
    let prepare_ns = started.elapsed().as_nanos() / prepare_calls as u128;

    let started = Instant::now();
    for op in 0..prepare_calls {
        let index = (op * 7919) % cfg.rows.max(1);
        let rows = conn
            .query_sql_with_params(POINT_SQL, &[Value::Text(key_at(index))])
            .map_err(|e| format!("profile unprepared point: {e}"))?;
        if rows.len() != 1 {
            return Err(format!("profile unprepared point returned {} rows", rows.len()));
        }
    }
    let unprepared_point_ns = started.elapsed().as_nanos() / prepare_calls as u128;

    let insert_calls = 200usize;
    let insert_start = cfg.rows + cfg.single_inserts;
    let started = Instant::now();
    for _ in 0..insert_calls {
        conn.prepare_sql(INSERT_SQL).map_err(|e| format!("profile insert prepare: {e}"))?;
    }
    let insert_prepare_ns = started.elapsed().as_nanos() / insert_calls as u128;

    conn.begin().map_err(|e| format!("profile insert begin: {e}"))?;
    let started = Instant::now();
    for index in insert_start..insert_start + insert_calls {
        yydb_insert(&conn, index, cfg)?;
    }
    let insert_row_ns = started.elapsed().as_nanos() / insert_calls as u128;
    let commit_started = Instant::now();
    conn.commit().map_err(|e| format!("profile insert commit: {e}"))?;
    let commit_per_batch_ns = commit_started.elapsed().as_nanos();

    conn.close();

    Ok(ProfileReport {
        rows: cfg.rows,
        range_len: cfg.range_len,
        calls,
        empty_window: StageReport::averaged(empty, calls),
        one_row: StageReport::averaged(single, calls),
        range_window: StageReport::averaged(window, calls),
        window_no_sort: StageReport::averaged(unsorted, calls),
        point_get: StageReport::averaged(point, point_calls),
        prepare_ns,
        unprepared_point_ns,
        insert_row_ns,
        insert_prepare_ns,
        commit_per_batch_ns,
    })
}

/// Rounds thrown away before the sample starts.
///
/// The first round pays for the harness itself: the binary is still being
/// paged in and both fixtures are read for the first time in this process.
/// Discarding exactly one round is a fixed rule, not a choice made after
/// looking at the numbers.
const DISCARDED_ROUNDS: usize = 1;

/// One round of both engines, measured back to back in the same process.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct MeasureRound {
    /// Zero-based index of the recorded round.
    pub round: usize,
    /// `cold` (fresh handles opened at round start) or `warm` (handles carried
    /// over from the previous round).
    pub cache: &'static str,
    /// Which engine ran first, so drift in machine state stays visible.
    pub order: &'static str,
    /// YYDB key-range scan, nanoseconds per returned row.
    pub yydb_range_ns_per_row: u128,
    /// SQLite key-range scan, nanoseconds per returned row.
    pub sqlite_range_ns_per_row: u128,
    /// YYDB point get, nanoseconds per operation.
    pub yydb_point_ns_per_op: u128,
    /// SQLite point get, nanoseconds per operation.
    pub sqlite_point_ns_per_op: u128,
}

/// Spread of one sample.
///
/// `median` is the lower median and `p90` uses the nearest-rank method, so a
/// seven-round run reports its largest value as the 90th percentile.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Series {
    /// Number of samples the spread was computed from.
    pub samples: usize,
    /// Smallest value.
    pub min: u128,
    /// Lower median.
    pub median: u128,
    /// 90th percentile by nearest rank.
    pub p90: u128,
    /// Largest value.
    pub max: u128,
}

impl Series {
    /// Summarises `values`, which may be empty.
    fn of(values: &[u128]) -> Self {
        if values.is_empty() {
            return Self::default();
        }
        let mut sorted = values.to_vec();
        sorted.sort_unstable();
        let last = sorted.len() - 1;
        let p90_index = ((sorted.len() as f64 * 0.9).ceil() as usize).saturating_sub(1).min(last);
        Self {
            samples: sorted.len(),
            min: sorted[0],
            median: sorted[last / 2],
            p90: sorted[p90_index],
            max: sorted[last],
        }
    }
}

/// Alternating same-process measurement of both engines.
///
/// SQLite's own per-row cost moves by more than a factor of two between
/// windows on this machine, so a ratio taken from two separate runs is not a
/// comparable number. This report keeps every round, alternates the engine
/// order, and toggles the engine page-cache state so absolute nanoseconds are
/// the primary evidence and the ratio is only a reference.
#[derive(Debug, Clone, Serialize)]
pub struct MeasureReport {
    /// Workload scale used for both engines.
    pub config: BenchConfig,
    /// Operating system the run happened on.
    pub os: &'static str,
    /// Rounds discarded before the sample started.
    pub discarded_rounds: usize,
    /// Every recorded round in order.
    pub rounds: Vec<MeasureRound>,
    /// YYDB key-range scan spread, nanoseconds per returned row.
    pub yydb_range: Series,
    /// SQLite key-range scan spread, nanoseconds per returned row.
    pub sqlite_range: Series,
    /// YYDB point get spread, nanoseconds per operation.
    pub yydb_point: Series,
    /// SQLite point get spread, nanoseconds per operation.
    pub sqlite_point: Series,
    /// `yydb / sqlite` per round in thousandths.
    pub ratio_x1000: Series,
}

fn companion_path(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut text = path.as_os_str().to_owned();
    text.push(suffix);
    std::path::PathBuf::from(text)
}

fn build_yydb_dataset(path: &Path, cfg: &BenchConfig) -> Result<(), String> {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(companion_path(path, suffix));
    }
    let conn = yydb::Connection::open_with_flags(path, yydb::OpenFlags::wal()).map_err(|e| format!("measure yydb open: {e}"))?;
    conn.ensure_schema("table Bench { @@id: utf8, payload: bytes }").map_err(|e| format!("measure yydb schema: {e}"))?;
    for chunk_start in (0..cfg.rows).step_by(cfg.batch_size.max(1)) {
        conn.begin().map_err(|e| format!("measure yydb begin: {e}"))?;
        for index in chunk_start..(chunk_start + cfg.batch_size.max(1)).min(cfg.rows) {
            yydb_insert(&conn, index, cfg)?;
        }
        conn.commit().map_err(|e| format!("measure yydb commit: {e}"))?;
    }
    verify_yydb_count(&conn, cfg.rows, "measure yydb fixture")?;
    conn.close();
    drop(conn);
    Ok(())
}

fn build_sqlite_dataset(path: &Path, cfg: &BenchConfig) -> Result<(), String> {
    let _ = std::fs::remove_file(path);
    let conn = rusqlite::Connection::open(path).map_err(|e| format!("measure sqlite open: {e}"))?;
    conn.execute_batch("PRAGMA journal_mode = WAL; CREATE TABLE Bench (id TEXT PRIMARY KEY, payload BLOB NOT NULL)")
        .map_err(|e| format!("measure sqlite schema: {e}"))?;
    for chunk_start in (0..cfg.rows).step_by(cfg.batch_size.max(1)) {
        conn.execute_batch("BEGIN").map_err(|e| format!("measure sqlite begin: {e}"))?;
        for index in chunk_start..(chunk_start + cfg.batch_size.max(1)).min(cfg.rows) {
            sqlite_insert(&conn, index, cfg)?;
        }
        conn.execute_batch("COMMIT").map_err(|e| format!("measure sqlite commit: {e}"))?;
    }
    let count: usize = conn
        .query_row("SELECT COUNT(*) FROM Bench", [], |row| row.get::<_, usize>(0))
        .map_err(|e| format!("measure sqlite count: {e}"))?;
    if count != cfg.rows {
        return Err(format!("measure sqlite fixture holds {count} rows, expected {}", cfg.rows));
    }
    drop(conn);
    Ok(())
}

fn open_yydb(path: &Path) -> Result<yydb::Connection, String> {
    yydb::Connection::open_with_flags(path, yydb::OpenFlags::wal()).map_err(|e| format!("measure yydb open: {e}"))
}

fn open_sqlite(path: &Path) -> Result<rusqlite::Connection, String> {
    rusqlite::Connection::open(path).map_err(|e| format!("measure sqlite open: {e}"))
}

/// Runs `cfg.measure_rounds` recorded rounds of both engines alternately.
///
/// Both fixtures are built once, then every round measures the identical
/// key-range scan and point-get workload. Even rounds open fresh handles so
/// the engine page cache starts empty. Odd rounds reuse the handles so the
/// cache holds the previous round's pages. Alternating the engine order keeps
/// a warming machine from always penalising the same engine.
pub fn measure(dir: &Path, cfg: &BenchConfig) -> Result<MeasureReport, String> {
    std::fs::create_dir_all(dir).map_err(|error| format!("cannot create measure dir: {error}"))?;
    let yydb_path = dir.join("measure.yydb");
    let sqlite_path = dir.join("measure.sqlite");
    build_yydb_dataset(&yydb_path, cfg)?;
    build_sqlite_dataset(&sqlite_path, cfg)?;

    let mut yydb_conn: Option<yydb::Connection> = Some(open_yydb(&yydb_path)?);
    let mut sqlite_conn: Option<rusqlite::Connection> = Some(open_sqlite(&sqlite_path)?);

    let mut rounds = Vec::new();
    let mut yydb_range = Vec::new();
    let mut sqlite_range = Vec::new();
    let mut yydb_point = Vec::new();
    let mut sqlite_point = Vec::new();
    let mut ratio = Vec::new();

    for sample in 0..(cfg.measure_rounds + DISCARDED_ROUNDS) {
        let cache = if sample % 2 == 0 { "cold" } else { "warm" };
        if cache == "cold" {
            // Close and drop before opening: two handles on one file would hold
            // two writer locks against each other.
            if let Some(conn) = yydb_conn.take() {
                conn.close();
            }
            drop(sqlite_conn.take());
            yydb_conn = Some(open_yydb(&yydb_path)?);
            sqlite_conn = Some(open_sqlite(&sqlite_path)?);
        }
        let yydb_handle = yydb_conn.as_ref().ok_or_else(|| "measure yydb handle missing".to_string())?;
        let sqlite_handle = sqlite_conn.as_ref().ok_or_else(|| "measure sqlite handle missing".to_string())?;
        // Order is driven by a separate bit from the cache state: tying both to
        // one parity would make every cold round run YYDB first and hand it a
        // systematically colder machine than SQLite.
        let yydb_first = (sample / 2) % 2 == 0;
        let order = if yydb_first { "yydb-first" } else { "sqlite-first" };
        let measured = if yydb_first {
            let yydb_range = yydb_range_pass(yydb_handle, cfg)?;
            let yydb_point = yydb_point_pass(yydb_handle, cfg, "measure yydb point")?;
            let sqlite_range = sqlite_range_pass(sqlite_handle, cfg)?;
            let sqlite_point = sqlite_point_pass(sqlite_handle, cfg, "measure sqlite point")?;
            (yydb_range, yydb_point, sqlite_range, sqlite_point)
        }
        else {
            let sqlite_range = sqlite_range_pass(sqlite_handle, cfg)?;
            let sqlite_point = sqlite_point_pass(sqlite_handle, cfg, "measure sqlite point")?;
            let yydb_range = yydb_range_pass(yydb_handle, cfg)?;
            let yydb_point = yydb_point_pass(yydb_handle, cfg, "measure yydb point")?;
            (yydb_range, yydb_point, sqlite_range, sqlite_point)
        };
        if sample < DISCARDED_ROUNDS {
            continue;
        }
        let (yydb_range_ns, yydb_point_ns, sqlite_range_ns, sqlite_point_ns) = measured;
        rounds.push(MeasureRound {
            round: sample - DISCARDED_ROUNDS,
            cache,
            order,
            yydb_range_ns_per_row: yydb_range_ns,
            sqlite_range_ns_per_row: sqlite_range_ns,
            yydb_point_ns_per_op: yydb_point_ns,
            sqlite_point_ns_per_op: sqlite_point_ns,
        });
        yydb_range.push(yydb_range_ns);
        sqlite_range.push(sqlite_range_ns);
        yydb_point.push(yydb_point_ns);
        sqlite_point.push(sqlite_point_ns);
        ratio.push(yydb_range_ns.saturating_mul(1000) / sqlite_range_ns.max(1));
    }

    if let Some(conn) = yydb_conn.take() {
        conn.close();
    }
    drop(sqlite_conn.take());

    Ok(MeasureReport {
        config: *cfg,
        os: std::env::consts::OS,
        discarded_rounds: DISCARDED_ROUNDS,
        rounds,
        yydb_range: Series::of(&yydb_range),
        sqlite_range: Series::of(&sqlite_range),
        yydb_point: Series::of(&yydb_point),
        sqlite_point: Series::of(&sqlite_point),
        ratio_x1000: Series::of(&ratio),
    })
}
