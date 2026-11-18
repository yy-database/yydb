// Blob publication ordering for `.yydx` volumes.
//
// A `.yydx` volume keeps its immutable segments in a sibling object directory,
// so the host must supply a directory port that answers "what is in there" and
// deletes a segment by name. OPFS only offers those through the asynchronous
// `values()` / `removeEntry()` pair, which the host resolves into the
// synchronous facade this binding calls.
//
// The publisher owns the ordering: a segment becomes durable on its own, and a
// crash between the segment publish and the main-file publish leaves a
// readable committed image plus one orphan segment. Nothing is rolled back by
// in-process bookkeeping.

use database::types::Error;
use yydb_wasm::{
    OpfsCapabilities, PersistentStorageMode, PersistentVolume, open_persistent_session_with_store,
    open_persistent_session_with_store_and_directory, validate_opfs_capabilities,
};

use crate::fixtures::{cleanup, fs_blob_directory, fs_store, temp_yydx};

fn main_image(tag: &str) -> Vec<u8> {
    let conn = database::execution::engine::Connection::open_in_memory().expect("memory");
    conn.put(tag, b"value").expect("seed write");
    conn.main_snapshot().expect("main snapshot")
}

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
fn yydb_opfs_yydx_session_open_is_refused_without_a_directory() {
    let path = temp_yydx("blob-refused");
    cleanup(&path);

    let error = open_persistent_session_with_store(path.to_str().unwrap(), fs_store(&path))
        .err()
        .expect("a `.yydx` session must not open without a blob directory");
    assert!(
        matches!(error, Error::NativeCapabilityMissing { capability: "opfs.blob_directory" }),
        "expected the `opfs.blob_directory` capability gate, got {error}"
    );
    assert!(!path.exists(), "a refused open must not create files");
    cleanup(&path);
}

#[test]
fn yydb_opfs_yydx_session_opens_with_a_directory() {
    let path = temp_yydx("blob-open");
    cleanup(&path);

    let mut session = open_persistent_session_with_store_and_directory(
        path.to_str().unwrap(),
        fs_store(&path),
        fs_blob_directory(&path),
    )
    .expect("a directory port opens the blob volume");
    assert!(!path.exists(), "opening an empty volume must not create a main file");
    session.close();
    cleanup(&path);
}

#[test]
fn yydb_opfs_crash_after_blob_publish_keeps_the_committed_image() {
    let path = temp_yydx("blob-crash");
    cleanup(&path);

    let committed = main_image("kept");
    {
        let volume =
            PersistentVolume::open_with_directory(path.to_str().unwrap(), fs_store(&path), fs_blob_directory(&path)).expect("open");
        volume.publish_main(&committed).expect("publish the committed image");
        volume.publish_blob("aaaa0001", b"segment").expect("publish one segment");
        // Simulated crash: the volume disappears with its handles, before any
        // main image that would reference the new segment is published.
    }

    let reopened =
        PersistentVolume::open_with_directory(path.to_str().unwrap(), fs_store(&path), fs_blob_directory(&path)).expect("reopen");
    assert_eq!(reopened.read_main().expect("read main"), Some(committed), "the committed image must survive");
    assert_eq!(
        reopened.list_blobs().expect("enumerate segments"),
        vec!["aaaa0001".to_owned()],
        "the segment published before the crash is still on the host"
    );
    assert_eq!(reopened.read_blob("aaaa0001").expect("read segment"), Some(b"segment".to_vec()));
    cleanup(&path);
}

#[test]
fn yydb_opfs_unpublished_blob_is_never_visible() {
    let path = temp_yydx("blob-unpublished");
    cleanup(&path);

    let volume = PersistentVolume::open_with_directory(path.to_str().unwrap(), fs_store(&path), fs_blob_directory(&path)).expect("open");
    assert_eq!(volume.list_blobs().expect("enumerate"), Vec::<String>::new(), "an empty volume holds no segments");
    assert_eq!(volume.read_blob("bbbb0002").expect("read"), None);

    volume.publish_blob("bbbb0002", b"body").expect("publish");
    volume.remove_blob("bbbb0002").expect("remove");
    assert_eq!(volume.list_blobs().expect("enumerate"), Vec::<String>::new(), "a removed segment is gone from the directory");
    assert_eq!(volume.read_blob("bbbb0002").expect("read"), None, "a removed segment must read as absent");
    cleanup(&path);
}

#[test]
fn yydb_opfs_blob_publish_refuses_a_different_body_for_the_same_hash() {
    let path = temp_yydx("blob-cas");
    cleanup(&path);

    let volume = PersistentVolume::open_with_directory(path.to_str().unwrap(), fs_store(&path), fs_blob_directory(&path)).expect("open");
    volume.publish_blob("cccc0003", b"first").expect("first publish");
    volume.publish_blob("cccc0003", b"first").expect("a repeated identical body is idempotent");
    let error = volume.publish_blob("cccc0003", b"other").expect_err("the same hash cannot carry a different body");
    assert!(matches!(error, Error::CasConflict { .. }), "expected CasConflict, got {error}");
    assert_eq!(volume.read_blob("cccc0003").expect("read"), Some(b"first".to_vec()));
    cleanup(&path);
}
