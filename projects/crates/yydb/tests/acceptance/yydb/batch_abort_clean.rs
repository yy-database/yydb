// fixture: yydb.batch.abort_clean

use crate::fixtures::yydb::{cleanup, open_temp_db, reopen};
use yydb::Batch;

#[test]
fn yydb_batch_abort_clean() {
    let (conn, path) = open_temp_db("batch-abort-clean");
    let mut batch = Batch::new();
    for i in 0..5 {
        batch.put(format!("abort/k{}", i), format!("v{}", i));
    }
    batch.abort();
    drop(conn);

    let conn = reopen(&path);
    for i in 0..5 {
        let key = format!("abort/k{}", i);
        assert_eq!(
            conn.get(&key).unwrap(),
            None,
            "fixture yydb.batch.abort_clean key {key} should be absent"
        );
    }
    cleanup(&path);
}
