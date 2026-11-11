use std::time::Duration;

use database::execution::engine::{Connection, EvictBudget, EvictionPolicy, NamespaceQuota};

use super::TempDb;

// fixture: yydb.ttl.robots_expiry
#[test]
fn yydb_ttl_robots_expiry() {
    let dir = TempDb::new("ttl-robots-expiry");
    let conn = Connection::open(dir.yydb()).expect("open yydb");
    conn.put_with_ttl("robots.txt/alpha", b"allow", Duration::from_millis(1)).expect("ttl put");
    std::thread::sleep(Duration::from_millis(10));

    let report = conn.evict_expired("robots.txt/", EvictBudget { max_records: 10 }).expect("evict expired");
    assert_eq!(report.evicted_records, 1);
    assert_eq!(conn.get("robots.txt/alpha").expect("get"), None);
}

// fixture: yydb.quota.response_evict
#[test]
fn yydb_quota_response_evict() {
    let dir = TempDb::new("quota-response-evict");
    let conn = Connection::open(dir.yydb()).expect("open yydb");
    conn.set_namespace_quota(
        "cache/response/",
        NamespaceQuota { max_bytes: u64::MAX, max_records: 2, eviction_policy: EvictionPolicy::Lru },
    )
    .expect("set quota");

    conn.put("cache/response/a", b"one").expect("put a");
    std::thread::sleep(Duration::from_millis(2));
    conn.put("cache/response/b", b"two").expect("put b");
    std::thread::sleep(Duration::from_millis(2));
    conn.put("cache/response/c", b"three").expect("put c");

    let stats = conn.namespace_stats("cache/response/").expect("stats");
    assert_eq!(stats.records_used, 2, "quota must bound the namespace record count");
    assert_eq!(conn.get("cache/response/a").expect("get evicted"), None, "LRU must evict the oldest record");
    assert_eq!(conn.get("cache/response/c").expect("get newest"), Some(b"three".to_vec()));
}
