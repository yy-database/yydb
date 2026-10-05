// fixture: yydb.lease.renew_extends

#[path = "../../fixtures/yydb/mod.rs"]
mod harness;
use std::time::Duration;

use harness::{cleanup, open_temp_db};
use yydb::LeaseExpectation;

#[test]
fn yydb_lease_renew_extends() {
    let (conn, path) = open_temp_db("lease-renew-extends");
    let token = conn
        .claim_lease(
            "frontier/item-2",
            "worker-a",
            Duration::from_secs(2),
            LeaseExpectation::Queued,
        )
        .unwrap();
    let initial = conn.lease_until_millis("frontier/item-2").unwrap().unwrap();
    let extended = initial + 5_000;
    conn.renew_lease(&token, extended).unwrap();
    assert_eq!(
        conn.lease_until_millis("frontier/item-2").unwrap(),
        Some(extended),
        "fixture yydb.lease.renew_extends lease_until not extended"
    );
    cleanup(&path);
}
