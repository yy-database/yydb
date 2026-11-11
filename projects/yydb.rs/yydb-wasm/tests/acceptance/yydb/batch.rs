use database::execution::engine::{Batch, Connection, DoctorSeverity};

use super::TempDb;

// fixture: yydb.batch.commit_visible
#[test]
fn yydb_batch_commit_visible() {
    let dir = TempDb::new("batch-commit-visible");
    let path = dir.yydb();
    {
        let conn = Connection::open(&path).expect("open yydb");
        let changes: Vec<(String, Option<Vec<u8>>)> =
            (0..20).map(|index| (format!("batch/k{index:02}"), Some(format!("v{index}").into_bytes()))).collect();
        conn.write_batch(&changes).expect("commit 20-key batch");
        conn.close();
    }

    let conn = Connection::open(&path).expect("reopen yydb");
    assert_eq!(conn.get("batch/k00").expect("get first key"), Some(b"v0".to_vec()));
    assert_eq!(conn.get("batch/k19").expect("get last key"), Some(b"v19".to_vec()));
    assert!(conn.scan_prefix("batch/", None, 64).expect("scan batch").len() >= 20);
}

// fixture: yydb.batch.abort_clean
#[test]
fn yydb_batch_abort_clean() {
    let dir = TempDb::new("batch-abort-clean");
    let path = dir.yydb();
    {
        let conn = Connection::open(&path).expect("open yydb");
        let mut batch = Batch::new();
        for index in 0..5 {
            batch.put(format!("abort/k{index}"), format!("v{index}"));
        }
        batch.abort();
        conn.close();
    }

    let conn = Connection::open(&path).expect("reopen yydb");
    assert_eq!(conn.get("abort/k0").expect("get aborted key"), None);
    assert_eq!(conn.get("abort/k4").expect("get aborted key"), None);
}

// fixture: yydb.batch.crash_mid_commit
#[test]
fn yydb_batch_crash_mid_commit() {
    let dir = TempDb::new("batch-crash-mid-commit");
    let path = dir.yydb();
    {
        let conn = Connection::open(&path).expect("open yydb");
        conn.put("seed", b"ok").expect("seed record");
        let mut batch = Batch::new();
        for index in 0..5 {
            batch.put(format!("crash/k{index}"), format!("v{index}"));
        }
        // Dropping the batch without `commit` is the crash-mid-commit cut: the
        // queued mutations never reach the journal.
        drop(batch);
        conn.close();
    }

    let conn = Connection::open(&path).expect("reopen yydb");
    assert_eq!(conn.get("seed").expect("seed survives"), Some(b"ok".to_vec()));
    assert_eq!(conn.get("crash/k0").expect("no leaked key"), None);
    let report = conn.doctor().expect("doctor");
    assert!(
        !report.issues.iter().any(|issue| issue.severity == DoctorSeverity::Error),
        "doctor reported errors after an uncommitted batch: {:?}",
        report.issues
    );
}
