use std::time::Duration;

use database::{
    execution::engine::{Batch, Connection, LeaseExpectation},
    types::Error,
};

use super::{TempDb, now_millis};

fn claim(conn: &Connection, key: &str, worker: &str, ttl: Duration) -> database::execution::engine::LeaseToken {
    conn.claim_lease(key, worker, ttl, LeaseExpectation::AbsentOrExpired).expect("claim lease")
}

// fixture: yydb.lease.claim_reclaim
#[test]
fn yydb_lease_claim_reclaim() {
    let dir = TempDb::new("lease-claim-reclaim");
    let path = dir.yydb();
    let stale = {
        let conn = Connection::open(&path).expect("open yydb");
        let token = claim(&conn, "frontier/item-1", "worker-a", Duration::from_millis(1));
        std::thread::sleep(Duration::from_millis(10));
        conn.close();
        token
    };

    let conn = Connection::open(&path).expect("reopen yydb");
    let current = claim(&conn, "frontier/item-1", "worker-b", Duration::from_secs(30));
    assert_eq!(
        conn.lease_until_millis("frontier/item-1").expect("read lease"),
        Some(current.lease_until),
        "worker-b must hold the reclaimed lease"
    );

    let mut batch = Batch::new();
    batch.put("frontier/item-1", b"stale-write");
    let error = conn.commit_with_lease(&stale, batch).expect_err("stale token must be fenced");
    assert!(matches!(error, Error::FencingMismatch { .. }), "expected FencingMismatch, got {error:?}");
    assert_eq!(conn.get("frontier/item-1").expect("get"), None, "fenced write must not land");

    // The active claim belongs to worker-b: its token still renews.
    conn.renew_lease(&current, now_millis() + 60_000).expect("worker-b renew");
    let error = conn.renew_lease(&stale, now_millis() + 60_000).expect_err("stale token must not renew");
    assert!(matches!(error, Error::FencingMismatch { .. }));
}

// fixture: yydb.lease.renew_extends
#[test]
fn yydb_lease_renew_extends() {
    let dir = TempDb::new("lease-renew-extends");
    let conn = Connection::open(dir.yydb()).expect("open yydb");
    let token = claim(&conn, "frontier/item-2", "worker-a", Duration::from_millis(50));
    let before = conn.lease_until_millis("frontier/item-2").expect("read lease").expect("lease present");

    conn.renew_lease(&token, before + 5_000).expect("renew lease");
    let after = conn.lease_until_millis("frontier/item-2").expect("read lease").expect("lease present");
    assert!(after >= before + 5_000, "renew must extend by at least 5000 ms, before={before} after={after}");
}

// fixture: yydb.lease.fencing_reject
#[test]
fn yydb_lease_fencing_reject() {
    let dir = TempDb::new("lease-fencing-reject");
    let conn = Connection::open(dir.yydb()).expect("open yydb");
    let stale = claim(&conn, "frontier/item-3", "worker-a", Duration::from_millis(1));
    std::thread::sleep(Duration::from_millis(10));
    claim(&conn, "frontier/item-3", "worker-b", Duration::from_secs(30));

    let mut batch = Batch::new();
    batch.put("frontier/item-3", b"blocked");
    let error = conn.commit_with_lease(&stale, batch).expect_err("superseded token must be rejected");
    assert!(matches!(error, Error::FencingMismatch { .. }), "expected FencingMismatch, got {error:?}");
}
