// Doctor coverage for a missing blob segment.
//
// The `yydb.doctor.missing_blob` issue is produced by blob volumes. This build
// refuses `.yydx` because the directory API is asynchronous (see
// `crash_after_blob_publish`), so a blob volume can never be opened here and the
// issue is unreachable through this binding. The case is kept explicit so the
// gap is recorded instead of silently dropped: doctor over blobs is covered
// upstream, where a blob directory exists.

use database::types::Error;
use yydb_wasm::{OpfsCapabilities, PersistentStorageMode, validate_opfs_capabilities};

use crate::fixtures::{USER_SCHEMA, assert_ok_true, cleanup, fs_store, temp_db};
use yydb_wasm::open_persistent_session_with_store;

#[test]
fn yydb_opfs_doctor_missing_blob_needs_a_blob_directory() {
    let error = validate_opfs_capabilities(&OpfsCapabilities::host_sync_handles(), PersistentStorageMode::Yydx)
        .expect_err("a blob volume cannot be opened by this binding");
    assert!(matches!(error, Error::NativeCapabilityMissing { .. }), "expected NativeCapabilityMissing, got {error}");
    assert!(error.to_string().contains("blob_directory"), "the refusal must name the missing primitive in {error}");
}

#[test]
fn yydb_opfs_doctor_reports_a_clean_single_file_volume() {
    let path = temp_db("doctor-clean");
    cleanup(&path);

    let mut session = open_persistent_session_with_store(path.to_str().unwrap(), fs_store(&path)).expect("open");
    assert_ok_true(&session.ensure_schema(USER_SCHEMA));
    assert_ok_true(&session.put("clean", b"value"));

    let report = session.doctor_json();
    assert!(report.contains("\"ok\":true"), "doctor over a healthy volume must succeed: {report}");
    session.close();

    cleanup(&path);
}
