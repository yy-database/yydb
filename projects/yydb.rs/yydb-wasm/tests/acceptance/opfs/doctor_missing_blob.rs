// Doctor coverage for blob volumes.
//
// `database.doctor.missing_blob` is produced by the engine's object store. A
// browser session hydrates an in-memory `Connection` from the published main
// image, so its CAS objects stay embedded in that image and the engine never
// consults a blob directory. Two consequences are recorded here instead of
// being silently dropped:
//
// - `.yydx` needs the directory port; without it the binding refuses the
//   volume with `opfs.blob_directory`.
// - A segment that disappears from the directory is reported by the port
//   itself (`list_blobs` / `read_blob`), not by a fabricated engine-side check.

use database::types::Error;
use yydb_wasm::{
    OpfsCapabilities, PersistentStorageMode, PersistentVolume, open_persistent_session_with_store_and_directory,
    validate_opfs_capabilities,
};

use crate::fixtures::{USER_SCHEMA, assert_ok_true, cleanup, fs_blob_directory, fs_store, temp_yydx};

#[test]
fn yydb_opfs_doctor_missing_blob_needs_a_blob_directory() {
    let error = validate_opfs_capabilities(&OpfsCapabilities::host_sync_handles(), PersistentStorageMode::Yydx)
        .expect_err("a blob volume cannot be opened by this binding");
    assert!(matches!(error, Error::NativeCapabilityMissing { .. }), "expected NativeCapabilityMissing, got {error}");
    assert!(error.to_string().contains("blob_directory"), "the refusal must name the missing primitive in {error}");
}

#[test]
fn yydb_opfs_doctor_reports_a_clean_yydx_volume() {
    let path = temp_yydx("doctor-yydx");
    cleanup(&path);

    let mut session =
        open_persistent_session_with_store_and_directory(path.to_str().unwrap(), fs_store(&path), fs_blob_directory(&path))
            .expect("open");
    assert_ok_true(&session.ensure_schema(USER_SCHEMA));
    assert_ok_true(&session.put("clean", b"value"));

    let report = session.doctor_json();
    assert!(report.contains("\"ok\":true"), "doctor over a healthy volume must succeed: {report}");
    assert!(!report.contains("missing_blob"), "a healthy volume must not report a missing blob: {report}");
    session.close();

    cleanup(&path);
}

#[test]
fn yydb_opfs_a_missing_segment_is_reported_by_the_directory_port() {
    let path = temp_yydx("doctor-missing-segment");
    cleanup(&path);

    {
        let volume =
            PersistentVolume::open_with_directory(path.to_str().unwrap(), fs_store(&path), fs_blob_directory(&path)).expect("open");
        volume.publish_blob("dddd0004", b"segment").expect("publish");
        assert_eq!(volume.list_blobs().expect("list"), vec!["dddd0004".to_owned()]);
    }

    // Out-of-band loss: the host directory no longer holds the segment.
    let root = database::storage::local_io::FsVolumeStore::open(&path).blob_root();
    std::fs::remove_file(root.join("dd").join("dddd0004.blob")).expect("drop the segment behind the volume's back");

    let reopened =
        PersistentVolume::open_with_directory(path.to_str().unwrap(), fs_store(&path), fs_blob_directory(&path)).expect("reopen");
    assert_eq!(reopened.list_blobs().expect("list"), Vec::<String>::new(), "the directory must not invent a segment");
    assert_eq!(reopened.read_blob("dddd0004").expect("read"), None, "a missing segment must read as absent");
    cleanup(&path);
}
