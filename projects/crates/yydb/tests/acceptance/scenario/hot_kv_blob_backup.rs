// fixture: yydb.scenario.hot_kv_blob_backup

use crate::fixtures::yydb::{cleanup, open_temp_yydx, temp_yydx};
use yydb::{Batch, Connection, ObjectKind, Tier};

#[test]
fn yydb_scenario_hot_kv_blob_backup() {
    let (conn, path) = open_temp_yydx("scenario-hot-kv-blob");
    let backup = temp_yydx("scenario-hot-kv-blob-copy");
    conn.ensure_schema("table Asset { @@id: uuid, name: utf8 }")
        .expect("schema");

    let payload: Vec<u8> = (0..4096_u16).map(|v| (v % 251) as u8).collect();
    let blob = conn
        .put_chunk(ObjectKind::Blob, &payload)
        .expect("stage blob chunk");

    let mut batch = Batch::new();
    batch.put("assets/status", b"hot");
    batch.attach_object(&blob, "assets/payload");
    batch.commit(&conn).expect("commit batch");

    conn.pin_object(&blob).expect("pin hot blob");
    assert_eq!(conn.object_tier(&blob), Tier::Hot);

    conn.backup_yydx_to(&backup).expect("backup yydx");
    drop(conn);

    let restored = Connection::open_yydx(&backup).expect("open backup");
    assert_eq!(
        restored.get("assets/status").expect("get status"),
        Some(b"hot".to_vec())
    );
    assert_eq!(
        restored.get_object(&blob).expect("read blob").as_ref(),
        payload.as_slice()
    );

    cleanup(&path);
    cleanup(&backup);
}
