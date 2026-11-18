// Blob publication ordering for `.yydx` volumes.
//
// `.yydx` needs the asynchronous directory API: enumerating and unlinking blob
// segments cannot be expressed with sync access handles, and the wasm surface
// runs synchronously. The binding therefore refuses the layout explicitly
// instead of emulating blob publication in process memory. Publish-ordering
// coverage for real blob volumes lives upstream, where a directory exists.

use database::types::Error;
use yydb_wasm::{OpfsCapabilities, PersistentStorageMode, open_persistent_session_with_store, validate_opfs_capabilities};

use crate::fixtures::{cleanup, fs_store, temp_db};

#[test]
fn yydb_opfs_blob_publication_requires_a_directory_api() {
    let error = validate_opfs_capabilities(&OpfsCapabilities::host_sync_handles(), PersistentStorageMode::Yydx)
        .expect_err("sync handles alone cannot serve a blob volume");
    assert!(
        matches!(error, Error::NativeCapabilityMissing { capability: "opfs.blob_directory" }),
        "expected the `opfs.blob_directory` capability gate, got {error}"
    );
}

#[test]
fn yydb_opfs_yydx_session_open_is_refused() {
    let path = temp_db("blob-refused");
    cleanup(&path);
    let yydx = path.with_extension("yydx");

    let error = open_persistent_session_with_store(yydx.to_str().unwrap(), fs_store(&yydx))
        .err()
        .expect("a `.yydx` session must not open without a blob directory");
    assert!(
        matches!(error, Error::NativeCapabilityMissing { capability: "opfs.blob_directory" }),
        "expected the `opfs.blob_directory` capability gate, got {error}"
    );
    assert!(!yydx.exists(), "a refused open must not create files");
    cleanup(&yydx);
}
