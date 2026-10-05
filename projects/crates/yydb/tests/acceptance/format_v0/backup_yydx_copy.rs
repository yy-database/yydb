// fixture: format_v0.backup_yydx_copy

use crate::fixtures::yydb::{cleanup, temp_yydx};
use yydb::{Batch, Connection, ObjectKind, ObjectStore};

#[test]
fn yydb_format_v0_backup_yydx_copy() {
    let path = temp_yydx("backup-yydx-src");
    let backup = temp_yydx("backup-yydx-copy");

    let conn = Connection::open_yydx(&path).expect("open");
    let live = conn.put_chunk(ObjectKind::Blob, b"payload").expect("put chunk");
    let mut batch = Batch::new();
    batch.attach_object(&live, "blobs/live");
    batch.commit(&conn).expect("commit");

    conn.backup_yydx_to(&backup).expect("backup");
    assert!(
        ObjectStore::yydx_objects_root(&backup).exists(),
        "backup must copy blob root"
    );
    drop(conn);

    let restored = Connection::open_yydx(&backup).expect("open backup");
    assert_eq!(
        restored.get("blobs/live").expect("get"),
        Some(format!("yydb:object:{}", live.hash_hex()).into_bytes())
    );
    assert_eq!(
        restored.get_object(&live).expect("read blob").as_ref(),
        b"payload"
    );

    cleanup(&path);
    cleanup(&backup);
}
