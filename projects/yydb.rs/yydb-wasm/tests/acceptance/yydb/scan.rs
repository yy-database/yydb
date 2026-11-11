use database::execution::engine::Connection;

use super::TempDb;

// fixture: yydb.scan.bounded_10k
#[test]
fn yydb_scan_bounded_10k() {
    let dir = TempDb::new("scan-bounded-10k");
    let conn = Connection::open(dir.yydb()).expect("open yydb");
    let changes: Vec<(String, Option<Vec<u8>>)> =
        (0..10_000).map(|index| (format!("scan/k{index:05}"), Some(b"x".to_vec()))).collect();
    conn.write_batch(&changes).expect("seed 10k keys");

    let first = conn.scan_prefix("scan/", None, 100).expect("first page");
    assert_eq!(first.len(), 100, "page must respect the 100-key limit");
    let cursor = first.last().expect("first page non-empty").0.clone();
    let second = conn.scan_prefix("scan/", Some(&cursor), 100).expect("second page");
    assert_eq!(second.len(), 100);
    assert!(second.first().expect("second page non-empty").0 > cursor, "pages must be disjoint and ordered");
}

// fixture: yydb.scan.resume_cursor
#[test]
fn yydb_scan_resume_cursor() {
    let dir = TempDb::new("scan-resume-cursor");
    let conn = Connection::open(dir.yydb()).expect("open yydb");
    let changes: Vec<(String, Option<Vec<u8>>)> =
        (0..250).map(|index| (format!("cursor/k{index:03}"), Some(b"indexed".to_vec()))).collect();
    conn.write_batch(&changes).expect("seed 250 keys");

    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..3 {
        let page = conn.scan_prefix("cursor/", cursor.as_deref(), 100).expect("scan page");
        cursor = page.last().map(|(key, _)| key.clone());
        seen.extend(page.into_iter().map(|(key, _)| key));
    }
    assert_eq!(seen.len(), 250, "three pages must cover every key");
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 250, "cursor resume must not duplicate keys");
}
