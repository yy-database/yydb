// fixture: format_v0.main_snapshot_file

use crate::fixtures::yydb::{cleanup, temp_db};
use yydb::{Connection, OpenFlags};
use yydb_format::PAGE_MAGIC;

#[test]
fn yydb_format_v0_main_snapshot_file_roundtrip() {
    let path = temp_db("main-snapshot-file");
    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).expect("open");
    conn.put("theme", b"dark").expect("put");

    let bytes = conn.main_snapshot().expect("snapshot");
    assert_eq!(&bytes[0..5], PAGE_MAGIC);

    let memory = Connection::open_in_memory().expect("memory");
    memory.load_main_snapshot(&bytes).expect("hydrate");
    assert_eq!(
        memory.get("theme").expect("get"),
        Some(b"dark".to_vec())
    );

    cleanup(&path);
}
