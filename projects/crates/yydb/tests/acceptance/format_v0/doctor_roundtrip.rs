//! format v0 doctor probes on a real `Connection`.

use std::fs;

use yydb::{Connection, DoctorSeverity, OpenFlags};
use yydb_format::diagnose_file;

fn temp_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "yydb-format-v0-doctor-{}-{}.yydb",
        std::process::id(),
        label
    ))
}

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../yydb-format/tests/fixtures/format-v0")
        .join(name)
}

#[test]
fn yydb_format_v0_doctor_roundtrip_clean() {
    let path = temp_path("clean");
    let _ = fs::remove_file(&path);

    let conn = Connection::open_with_flags(&path, OpenFlags::new()).unwrap();
    conn.put("gate/key", b"ok").unwrap();
    conn.checkpoint().unwrap();
    drop(conn);

    let reopened = Connection::open(&path).unwrap();
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

#[test]
fn yydb_format_v0_doctor_corrupt_header_fixture() {
    let path = temp_path("bad-header");
    let _ = fs::remove_file(&path);
    fs::copy(fixture("page0_bad_checksum.bin"), &path).unwrap();

    let issues = diagnose_file(&path).unwrap();
    let slot_a = issues.iter().find(|issue| {
        issue.code == "yydb.doctor.corrupt_header" && issue.key_hint.as_deref() == Some("slot_a")
    });
    assert!(
        slot_a.is_some(),
        "expected slot_a corrupt_header issue, got {:?}",
        issues
    );
    let _ = fs::remove_file(&path);
}
