// gate: G-YYDB-3
// fixture: yydb.lease.claim_reclaim

mod common;

use std::thread;
use std::time::Duration;

use common::{cleanup, open_temp_db, reopen};
use yydb::{Batch, Error, LeaseExpectation};

#[test]
fn g_yydb_lease_claim_reclaim() {
    let (conn, path) = open_temp_db("lease-claim-reclaim");
    let token_a = conn
        .claim_lease(
            "frontier/item-1",
            "worker-a",
            Duration::from_millis(1),
            LeaseExpectation::AbsentOrExpired,
        )
        .unwrap();
    thread::sleep(Duration::from_millis(10));
    drop(conn);

    let conn = reopen(&path);
    let token_b = conn
        .claim_lease(
            "frontier/item-1",
            "worker-b",
            Duration::from_secs(30),
            LeaseExpectation::AbsentOrExpired,
        )
        .unwrap();
    assert_eq!(token_b.worker_id, "worker-b");

    let mut batch = Batch::new();
    batch.put("frontier/item-1", b"stale-write");
    let err = conn.commit_with_lease(&token_a, batch).unwrap_err();
    assert!(
        matches!(err, Error::FencingMismatch { .. }),
        "gate G-YYDB-3 fixture yydb.lease.claim_reclaim expected FencingMismatch, got {err:?}"
    );
    cleanup(&path);
}
