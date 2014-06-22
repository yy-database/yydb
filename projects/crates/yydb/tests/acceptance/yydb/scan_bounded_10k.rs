// fixture: yydb.scan.bounded_10k

#[path = "../../fixtures/yydb/mod.rs"]
mod harness;
use harness::{cleanup, open_temp_db};

#[test]
fn yydb_scan_bounded_10k() {
    let (conn, path) = open_temp_db("scan-bounded-10k");
    let changes: Vec<(String, Option<Vec<u8>>)> = (0..10_000)
        .map(|i| (format!("scan/k{:05}", i), Some(b"x".to_vec())))
        .collect();
    conn.write_batch(&changes).unwrap();

    let page = conn.scan_prefix("scan/", None, 100).unwrap();
    assert_eq!(
        page.len(),
        100,
        "fixture yydb.scan.bounded_10k first page should respect limit"
    );

    let cursor = page.last().map(|(key, _)| key.clone());
    let next = conn.scan_prefix("scan/", cursor.as_deref(), 100).unwrap();
    assert_eq!(next.len(), 100);
    assert_ne!(page[0].0, next[0].0);
    cleanup(&path);
}
