// gate: G-YYDB-1
// fixture: yydb.batch.commit_visible

mod common;

use common::{cleanup, open_temp_db, reopen};

#[test]
fn g_yydb_batch_commit_visible() {
    let (conn, path) = open_temp_db("batch-commit-visible");
    let changes: Vec<(String, Option<Vec<u8>>)> = (0..20)
        .map(|i| (format!("batch/k{:02}", i), Some(format!("v{}", i).into_bytes())))
        .collect();
    conn.write_batch(&changes).unwrap();
    drop(conn);

    let conn = reopen(&path);
    for i in 0..20 {
        let key = format!("batch/k{:02}", i);
        assert_eq!(
            conn.get(&key).unwrap(),
            Some(format!("v{}", i).into_bytes()),
            "gate G-YYDB-1 fixture yydb.batch.commit_visible missing key {key}"
        );
    }
    cleanup(&path);
}
