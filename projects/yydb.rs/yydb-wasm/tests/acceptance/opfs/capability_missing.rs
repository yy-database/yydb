// OPFS capability gate: a host that supplies no sync handles must be refused.

use database::types::Error;
use wasm_bindgen::JsValue;
use yydb_wasm::{OpfsCapabilities, PersistentStorageMode, open_volume_from_handles, validate_opfs_capabilities};

fn assert_missing(result: Result<(), Error>, capability: &str) {
    match result {
        Err(Error::NativeCapabilityMissing { capability: found }) => {
            assert_eq!(found, capability, "expected `{capability}` in {found}")
        }
        other => panic!("expected NativeCapabilityMissing `{capability}`, got {other:?}"),
    }
}

#[test]
fn yydb_opfs_capability_missing_without_sync_handles() {
    let caps = OpfsCapabilities::unavailable();
    assert_missing(validate_opfs_capabilities(&caps, PersistentStorageMode::Yydb), "opfs.sync_access_handle");
}

#[test]
fn yydb_opfs_capability_missing_without_durability_barrier() {
    let caps = OpfsCapabilities { durability_sync: false, ..OpfsCapabilities::host_sync_handles() };
    assert_missing(validate_opfs_capabilities(&caps, PersistentStorageMode::Yydb), "opfs.durability_sync");
}

#[test]
fn yydb_opfs_capability_missing_for_yydx_blob_directory() {
    let caps = OpfsCapabilities::host_sync_handles();
    assert_missing(validate_opfs_capabilities(&caps, PersistentStorageMode::Yydx), "opfs.blob_directory");
}

#[test]
fn yydb_opfs_volume_open_without_handles_reports_the_missing_capability() {
    let error = match open_volume_from_handles("probe-app.yydb", JsValue::UNDEFINED) {
        Ok(_) => panic!("a volume must not open without host sync handles"),
        Err(error) => error,
    };
    assert!(matches!(error, Error::NativeCapabilityMissing { .. }), "expected NativeCapabilityMissing, got {error}");
    assert!(error.to_string().contains("opfs."), "capability label must be reported in {error}");
}

#[test]
fn yydb_opfs_diagnostic_carries_the_capability_param() {
    let error = validate_opfs_capabilities(&OpfsCapabilities::unavailable(), PersistentStorageMode::Yydb).unwrap_err();
    let json = error.to_json();
    assert_eq!(json.get("code"), Some(&serde_json::json!("database.native_capability_missing")));
    assert_eq!(json.pointer("/params/capability"), Some(&serde_json::json!("opfs.sync_access_handle")));
}
