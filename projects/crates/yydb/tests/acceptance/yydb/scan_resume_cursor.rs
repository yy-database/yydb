// fixture: yydb.scan.resume_cursor

use crate::fixtures::yydb::{cleanup, open_temp_db};

#[test]
fn yydb_scan_resume_cursor() {
    let (conn, path) = open_temp_db("scan-resume-cursor");
    let changes: Vec<(String, Option<Vec<u8>>)> = (0..250)
        .map(|i| (format!("cursor/k{:03}", i), Some(vec![i as u8])))
        .collect();
    conn.write_batch(&changes).unwrap();

    let mut seen = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..3 {
        let page = conn.scan_prefix("cursor/", after.as_deref(), 100).unwrap();
        assert!(!page.is_empty());
        for (key, _) in &page {
            assert!(
                !seen.contains(key),
                "fixture yydb.scan.resume_cursor duplicate key {key}"
            );
            seen.push(key.clone());
        }
        after = page.last().map(|(key, _)| key.clone());
    }
    assert_eq!(seen.len(), 250);
    cleanup(&path);
}
