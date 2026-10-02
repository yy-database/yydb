// gate: G-YYDB-3
// fixture: yydb.lease.renew_extends

mod common;

use std::time::Duration;

use common::{cleanup, open_temp_db};
use yydb::LeaseExpectation;

#[test]
fn g_yydb_lease_renew_extends() {
    let (conn, path) = open_temp_db("lease-renew-extends");
    let token = conn
        .claim_lease(
            "frontier/item-2",
            "worker-a",
            Duration::from_millis(50),
            LeaseExpectation::Queued,
        )
        .unwrap();
    let initial = conn.lease_until_millis("frontier/item-2").unwrap().unwrap();
    let extended = initial + 5_000;
    conn.renew_lease(&token, extended).unwrap();
    assert_eq!(
        conn.lease_until_millis("frontier/item-2").unwrap(),
        Some(extended),
        "gate G-YYDB-3 fixture yydb.lease.renew_extends lease_until not extended"
    );
    cleanup(&path);
}
