// fixture: yydb.ttl.robots_expiry

use std::thread;
use std::time::Duration;

use crate::fixtures::yydb::{cleanup, open_temp_db};
use yydb::EvictBudget;

#[test]
fn yydb_ttl_robots_expiry() {
    let (conn, path) = open_temp_db("ttl-robots-expiry");
    conn.put_with_ttl("robots.txt/alpha", b"allow", Duration::from_millis(1))
        .unwrap();
    thread::sleep(Duration::from_millis(10));

    let report = conn
        .evict_expired("robots.txt/", EvictBudget { max_records: 10 })
        .unwrap();
    assert_eq!(
        report.evicted_records, 1,
        "fixture yydb.ttl.robots_expiry expected one eviction"
    );
    assert_eq!(conn.get("robots.txt/alpha").unwrap(), None);
    cleanup(&path);
}
