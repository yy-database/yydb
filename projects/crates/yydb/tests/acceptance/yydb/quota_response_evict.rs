// fixture: yydb.quota.response_evict

use std::thread;
use std::time::Duration;

use crate::fixtures::yydb::{cleanup, open_temp_db};
use yydb::{EvictionPolicy, NamespaceQuota};

#[test]
fn yydb_quota_response_evict() {
    let (conn, path) = open_temp_db("quota-response-evict");
    conn.set_namespace_quota(
        "cache/response/",
        NamespaceQuota {
            max_bytes: 10_000,
            max_records: 2,
            eviction_policy: EvictionPolicy::Lru,
        },
    )
    .unwrap();

    conn.put("cache/response/a", b"one").unwrap();
    thread::sleep(Duration::from_millis(1));
    conn.put("cache/response/b", b"two").unwrap();
    thread::sleep(Duration::from_millis(1));
    conn.put("cache/response/c", b"three").unwrap();

    let stats = conn.namespace_stats("cache/response/").unwrap();
    assert_eq!(
        stats.records_used, 2,
        "fixture yydb.quota.response_evict expected LRU eviction to two records"
    );
    assert_eq!(conn.get("cache/response/a").unwrap(), None);
    assert_eq!(
        conn.get("cache/response/c").unwrap(),
        Some(b"three".to_vec())
    );
    cleanup(&path);
}
