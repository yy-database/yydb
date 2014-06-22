// fixture: yydb.opfs.quota_exhausted

use yydb::Error;
use yydb_wasm::{claim_opfs_writer, OpfsCapabilities, OpfsCommittedVolume};

#[test]
fn yydb_opfs_quota_exhausted() {
    let caps = OpfsCapabilities::full_yydb_profile();
    let _writer = claim_opfs_writer("app.yydb", &caps).expect("writer lease");

    let mut volume = OpfsCommittedVolume::new("app.yydb", b"generation-1", 32);
    assert_eq!(volume.read_committed(), b"generation-1");

    let oversized = b"generation-2-with-too-many-bytes-for-quota";
    let error = volume.publish_generation(oversized).unwrap_err();
    assert!(
        matches!(error, Error::QuotaExceeded { .. }),
        "expected QuotaExceeded, got {error}"
    );
    assert_eq!(
        volume.read_committed(),
        b"generation-1",
        "committed generation must remain readable after aborted publish"
    );
}

#[test]
fn yydb_opfs_quota_exhausted_allows_small_publish() {
    let mut volume = OpfsCommittedVolume::new("app.yydb", b"gen-1", 64);
    volume.publish_generation(b"gen-2").expect("fits quota");
    assert_eq!(volume.read_committed(), b"gen-2");
}
