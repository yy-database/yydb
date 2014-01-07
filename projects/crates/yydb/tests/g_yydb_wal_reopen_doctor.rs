// gate: G-YYDB-7
// fixture: yydb.wal.reopen_doctor

mod common;

use common::{cleanup, open_temp_db, reopen};
use yydb::{journal::wal_path, OpenFlags};

#[test]
fn g_yydb_wal_reopen_doctor() {
    let (_unused, path) = open_temp_db("wal-reopen");
    cleanup(&path);
    let conn = yydb::Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    conn.put("wal/a", b"1").unwrap();
    conn.put("wal/b", b"2").unwrap();
    assert!(wal_path(&path).exists());
    drop(conn);

    let reopened = reopen(&path);
    assert_eq!(reopened.get("wal/a").unwrap(), Some(b"1".to_vec()));
    assert_eq!(reopened.get("wal/b").unwrap(), Some(b"2".to_vec()));
    reopened.checkpoint().unwrap();
    cleanup(&path);
}
