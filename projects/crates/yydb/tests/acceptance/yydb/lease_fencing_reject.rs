// fixture: yydb.lease.fencing_reject

#[path = "../../fixtures/yydb/mod.rs"]
mod harness;
use std::thread;
use std::time::Duration;

use harness::{cleanup, open_temp_db};
use yydb::{Batch, Error, LeaseExpectation};

#[test]
fn yydb_lease_fencing_reject() {
    let (conn, path) = open_temp_db("lease-fencing-reject");
    let token_a = conn
        .claim_lease(
            "frontier/item-3",
            "worker-a",
            Duration::from_millis(1),
            LeaseExpectation::AbsentOrExpired,
        )
        .unwrap();
    thread::sleep(Duration::from_millis(10));
    conn.claim_lease(
        "frontier/item-3",
        "worker-b",
        Duration::from_secs(30),
        LeaseExpectation::AbsentOrExpired,
    )
    .unwrap();

    let mut batch = Batch::new();
    batch.put("frontier/item-3", b"blocked");
    let err = conn.commit_with_lease(&token_a, batch).unwrap_err();
    assert!(
        matches!(err, Error::FencingMismatch { .. }),
        "fixture yydb.lease.fencing_reject expected FencingMismatch, got {err:?}"
    );
    cleanup(&path);
}
