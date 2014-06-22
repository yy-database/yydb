// fixture: yydb.wal.reopen_doctor

#[path = "../../fixtures/yydb/mod.rs"]
mod harness;
use harness::{cleanup, reopen, temp_db_path};
use yydb::{journal::wal_path, DoctorSeverity, OpenFlags};

#[test]
fn yydb_wal_reopen_doctor() {
    let path = temp_db_path("wal-reopen");
    cleanup(&path);
    let conn = yydb::Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    conn.put("wal/a", b"1").unwrap();
    conn.put("wal/b", b"2").unwrap();
    assert!(wal_path(&path).exists());
    drop(conn);

    let reopened = reopen(&path);
    assert_eq!(reopened.get("wal/a").unwrap(), Some(b"1".to_vec()));
    assert_eq!(reopened.get("wal/b").unwrap(), Some(b"2".to_vec()));
    let report = reopened.doctor().unwrap();
    assert!(
        !report
            .issues
            .iter()
            .any(|issue| issue.severity == DoctorSeverity::Error),
        "fixture yydb.wal.reopen_doctor doctor reported errors: {:?}",
        report.issues
    );
    reopened.checkpoint().unwrap();
    cleanup(&path);
}
