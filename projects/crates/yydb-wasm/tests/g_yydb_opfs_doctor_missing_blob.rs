// gate: G-OPFS-6
// fixture: yydb.opfs.doctor_missing_blob
//
// Living `08` §5 / `fixtures/g-yydb/cases/yydb_opfs_doctor_missing_blob.json`.

use yydb::DoctorSeverity;
use yydb_wasm::{opfs_commit_and_sync, opfs_doctor, opfs_evict_blob, OpfsBlobPublication};

#[test]
fn g_yydb_opfs_doctor_missing_blob() {
    let path = "app.yydx";
    let mut publication = OpfsBlobPublication::new(path, &["blob-a", "blob-b"]);
    opfs_commit_and_sync(path, &mut publication, &["blob-a", "blob-b"], b"catalog-v1")
        .expect("durable commit");
    opfs_evict_blob(path, "blob-b");

    let report = opfs_doctor(path).expect("doctor must return a report");
    let missing = report
        .issues
        .iter()
        .find(|issue| issue.code == "yydb.doctor.missing_blob");
    assert!(
        missing.is_some(),
        "gate G-OPFS-6 fixture yydb.opfs.doctor_missing_blob expected missing_blob issue, got {:?}",
        report.issues
    );
    assert_eq!(missing.unwrap().severity, DoctorSeverity::Error);
    assert_eq!(missing.unwrap().key_hint.as_deref(), Some("blob-b"));
}
