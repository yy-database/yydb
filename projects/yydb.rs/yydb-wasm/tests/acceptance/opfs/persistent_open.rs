// Persistent open: the upstream publisher over a real host byte store.

use crate::fixtures::{cleanup, fs_store, temp_db};
use yydb_wasm::{OpfsCapabilities, open_persistent_session_with_store, validate_opfs_capabilities};

#[test]
fn yydb_opfs_host_profile_accepts_a_yydb_volume() {
    validate_opfs_capabilities(&OpfsCapabilities::host_sync_handles(), yydb_wasm::PersistentStorageMode::Yydb)
        .expect("host sync handles satisfy the .yydb profile");
}

#[test]
fn yydb_opfs_persistent_session_reads_an_empty_volume() {
    let path = temp_db("open-empty");
    cleanup(&path);

    let mut session =
        open_persistent_session_with_store(path.to_str().unwrap(), fs_store(&path)).expect("open over byte store");
    assert_eq!(session.path(), path.to_str().unwrap());
    assert!(!path.exists(), "opening an empty volume must not create a main file");
    session.close();
    cleanup(&path);
}

#[test]
fn yydb_opfs_persistent_session_rejects_a_path_without_a_layout_suffix() {
    let error = open_persistent_session_with_store("probe-app.txt", fs_store(&temp_db("open-suffix")))
        .err()
        .expect("unsupported layout must fail");
    assert!(matches!(error, database::types::Error::Unsupported(_)), "expected Unsupported, got {error}");
}
