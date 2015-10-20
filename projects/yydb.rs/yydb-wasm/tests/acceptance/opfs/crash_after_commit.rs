// fixture: yydb.opfs.crash_after_commit

use yydb_wasm::{OpfsBlobPublication, opfs_commit_and_sync, opfs_reopen};

#[test]
fn yydb_opfs_crash_after_commit() {
    let path = "crash-after-commit.yydx";
    let mut publication = OpfsBlobPublication::new(path, &["blob-a"]);
    publication.publish_blob("blob-b");

    let committed = opfs_commit_and_sync(path, &mut publication, &["blob-a", "blob-b"], b"catalog-v2").expect("durable commit");
    // Crash after `TxnCommit` sync: durable snapshot must already be complete.

    let reopened = opfs_reopen(path).expect("reopen after crash");
    assert_eq!(reopened, committed);
    assert_eq!(reopened.catalog_body, b"catalog-v2");
    assert_eq!(reopened.visible_references(), vec!["blob-a".to_string(), "blob-b".to_string()]);
    assert!(publication.dangling_references().is_empty());
}
