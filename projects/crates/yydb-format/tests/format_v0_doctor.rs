//! format v0 doctor probes against golden fixtures.

use std::path::PathBuf;

use yydb_format::diagnose_file;
use yydb_types::DoctorSeverity;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/format-v0")
        .join(name)
}

#[test]
fn format_v0_doctor_page0_empty_clean() {
    let path = fixture("page0_empty_dual_slot.bin");
    let issues = diagnose_file(&path).unwrap();
    assert!(
        issues
            .iter()
            .all(|issue| issue.severity != DoctorSeverity::Error),
        "unexpected doctor errors: {:?}",
        issues
    );
}

#[test]
fn format_v0_doctor_page0_bad_checksum_warns_slot_a() {
    let path = fixture("page0_bad_checksum.bin");
    let issues = diagnose_file(&path).unwrap();
    assert!(
        issues.iter().any(|issue| {
            issue.code == "yydb.doctor.corrupt_header"
                && issue.severity == DoctorSeverity::Warn
                && issue.key_hint.as_deref() == Some("slot_a")
        }),
        "expected slot_a warn, got {:?}",
        issues
    );
}
