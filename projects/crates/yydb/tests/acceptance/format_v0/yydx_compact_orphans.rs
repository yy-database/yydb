// fixture: format_v0.yydx_compact_orphans

use crate::fixtures::yydb::{cleanup, temp_yydx};
use yydb::{journal::wal_path, Batch, Connection, ObjectKind, OpenFlags};

#[test]
fn yydb_format_v0_yydx_compact_orphans() {
    let path = temp_yydx("yydx-compact");
    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).expect("wal open");

    let live = conn.put_chunk(ObjectKind::Blob, b"keep").expect("live chunk");
    let mut batch = Batch::new();
    batch.attach_object(&live, "pages/kept");
    batch.commit(&conn).expect("commit");

    conn.put_chunk(ObjectKind::Blob, b"drop-a").expect("orphan a");
    conn.put_chunk(ObjectKind::Blob, b"drop-b").expect("orphan b");
    assert!(wal_path(&path).exists(), "wal sidecar should exist before compact");

    let report = conn.compact_yydx().expect("compact");
    assert!(report.checkpointed);
    assert_eq!(report.orphans_reclaimed, 2);
    assert!(
        !wal_path(&path).exists(),
        "compact must fold wal into the main file"
    );

    let doctor = conn.doctor().expect("doctor");
    assert_eq!(doctor.orphan_object_count, 0);
    assert!(conn.get_object(&live).is_ok());

    cleanup(&path);
}

#[test]
fn yydb_format_v0_yydx_compact_skips_pinned_orphans() {
    let path = temp_yydx("yydx-compact-pinned");
    let conn = Connection::open_yydx(&path).expect("open");

    let pinned = conn
        .put_chunk(ObjectKind::Blob, b"pinned-orphan")
        .expect("orphan");
    conn.pin_object(&pinned).expect("pin");
    conn.put_chunk(ObjectKind::Blob, b"drop").expect("orphan");

    let report = conn.compact_yydx().expect("compact");
    assert_eq!(report.orphans_reclaimed, 1);
    assert!(conn.get_object(&pinned).is_ok());

    conn.evict_object(&pinned).expect("evict");
    let second = conn.compact_yydx().expect("compact again");
    assert_eq!(second.orphans_reclaimed, 1);
    assert!(conn.get_object(&pinned).is_err());

    cleanup(&path);
}
