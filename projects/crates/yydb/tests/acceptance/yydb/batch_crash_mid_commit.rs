// fixture: yydb.batch.crash_mid_commit

#[path = "../../fixtures/yydb/mod.rs"]
mod harness;
use harness::{cleanup, open_temp_db, reopen};
use yydb::{Batch, DoctorSeverity};

#[test]
fn yydb_batch_crash_mid_commit() {
    let (conn, path) = open_temp_db("batch-crash-mid-commit");
    conn.put("seed", b"ok").unwrap();

    let mut batch = Batch::new();
    for index in 0..5 {
        batch.put(format!("crash/k{}", index), format!("v{}", index));
    }
    // Simulate a crash before commit by dropping the in-memory batch.
    drop(batch);
    drop(conn);

    let conn = reopen(&path);
    for index in 0..5 {
        let key = format!("crash/k{}", index);
        assert_eq!(
            conn.get(&key).unwrap(),
            None,
            "fixture yydb.batch.crash_mid_commit key {key} should be absent"
        );
    }

    let report = conn.doctor().unwrap();
    assert!(
        !report
            .issues
            .iter()
            .any(|issue| issue.severity == DoctorSeverity::Error),
        "fixture yydb.batch.crash_mid_commit doctor reported errors: {:?}",
        report.issues
    );
    cleanup(&path);
}
