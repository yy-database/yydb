// fixture: yydb.scenario.yydx_compact_backup

use crate::fixtures::yydb::{cleanup, temp_yydx};
use yydb::{Batch, Connection, ObjectKind, OpenFlags};

#[test]
fn yydb_scenario_yydx_compact_backup() {
    let path = temp_yydx("scenario-compact-backup");
    let backup = temp_yydx("scenario-compact-backup-copy");

    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).expect("open");
    let live = conn.put_chunk(ObjectKind::Blob, b"payload").expect("blob");
    let mut batch = Batch::new();
    batch.attach_object(&live, "assets/live");
    batch.commit(&conn).expect("commit");

    conn.put_chunk(ObjectKind::Blob, b"orphan").expect("orphan");
    let compact = conn.compact_yydx().expect("compact");
    assert_eq!(compact.orphans_reclaimed, 1);
    assert_eq!(conn.doctor().expect("doctor").orphan_object_count, 0);

    conn.backup_yydx_to(&backup).expect("backup");
    drop(conn);

    let restored = Connection::open_yydx(&backup).expect("restore");
    assert_eq!(
        restored.get("assets/live").expect("get"),
        Some(format!("yydb:object:{}", live.hash_hex()).into_bytes())
    );
    assert_eq!(
        restored.get_object(&live).expect("read blob").as_ref(),
        b"payload"
    );

    cleanup(&path);
    cleanup(&backup);
}
