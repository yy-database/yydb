// gate: G-OPFS-4
// fixture: yydb.opfs.crash_after_blob_publish
//
// Living `07` §13 / `08` / `fixtures/g-yydb/cases/yydb_opfs_crash_after_blob_publish.json`.

use yydb::Error;
use yydb_wasm::OpfsBlobPublication;

#[test]
fn g_yydb_opfs_crash_after_blob_publish() {
    let mut volume = OpfsBlobPublication::new("app.yydx", &["blob-live"]);
    volume.publish_blob("blob-orphan");
    // Crash between blob publish (step 2) and catalog commit (steps 3–4).

    assert_eq!(volume.visible_references(), vec!["blob-live".to_string()]);
    assert_eq!(volume.orphan_blobs(), vec!["blob-orphan".to_string()]);
    assert!(
        volume.dangling_references().is_empty(),
        "committed catalog must not reference unpublished blobs"
    );
}

#[test]
fn g_yydb_opfs_commit_rejects_unpublished_blob_reference() {
    let mut volume = OpfsBlobPublication::new("app.yydx", &[]);
    let error = volume.commit_catalog(&["blob-missing"]).unwrap_err();
    assert!(matches!(error, Error::ObjectNotFound { .. }));
    assert!(volume.dangling_references().is_empty());
}
