//! gate: format-v0 (Living 09)
//! fixture: format_v0.roundtrip

use std::fs;

use yydb::{Connection, DoctorSeverity, OpenFlags};

fn temp_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "g-yydb-format-v0-roundtrip-{}-{}.yydb",
        std::process::id(),
        label
    ))
}

#[test]
fn g_yydb_format_v0_roundtrip() {
    let path = temp_path("roundtrip");
    let _ = fs::remove_file(&path);

    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    conn.put("fixture/one", b"value").unwrap();
    conn.checkpoint().unwrap();
    drop(conn);

    let reopened = Connection::open(&path).unwrap();
    assert_eq!(
        reopened.get("fixture/one").unwrap(),
        Some(b"value".to_vec())
    );
    let report = reopened.doctor().unwrap();
    assert!(
        report
            .issues
            .iter()
            .all(|issue| issue.severity != DoctorSeverity::Error),
        "doctor reported errors: {:?}",
        report.issues
    );
    let _ = fs::remove_file(&path);
}
