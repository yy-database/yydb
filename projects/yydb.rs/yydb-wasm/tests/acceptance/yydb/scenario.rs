//! End-to-end scenarios over the `.yydx` multi-file layout.
//!
//! Each case composes real primitives (KV batch, CAS blobs, VOS transactions,
//! backup and restore) and asserts values written by the test itself.

use database::execution::engine::{Batch, Connection, ObjectKind, Tier, Value};

use super::TempDb;

const LARGE_BLOB_BYTES: usize = 64 * 1024;

const USER_SCHEMA: &str = r#"
table User {
    @@user_id: uuid,
    user_name: utf8,
    active: bool,
}
"#;

fn large_payload(seed: u8) -> Vec<u8> {
    (0..LARGE_BLOB_BYTES).map(|index| seed.wrapping_add((index % 251) as u8)).collect()
}

fn backup_path(main: &std::path::Path) -> std::path::PathBuf {
    main.with_file_name("backup.yydx")
}

// fixture: yydb.scenario.hot_kv_blob_backup
#[test]
fn yydb_scenario_hot_kv_blob_backup() {
    let dir = TempDb::new("scenario-hot-kv-blob-backup");
    let live_path = dir.yydx();
    let backup = backup_path(&live_path);
    let payload = large_payload(17);

    let blob = {
        let conn = Connection::open_yydx(&live_path).expect("open yydx");
        conn.ensure_schema("table Asset { @@id: uuid, name: utf8 }").expect("ensure schema");

        let blob = conn.put_chunk(ObjectKind::Blob, &payload).expect("store large blob");

        let mut hot = Batch::new();
        hot.put("assets/status", "hot");
        hot.commit(&conn).expect("commit hot kv batch");

        let mut attachment = Batch::new();
        attachment.attach_object(&blob, "assets/payload");
        attachment.commit(&conn).expect("commit blob attachment");

        conn.pin_object(&blob).expect("pin blob");
        assert_eq!(conn.object_tier(&blob), Tier::Hot, "a pinned blob must report the hot tier");
        conn.backup_yydx_to(&backup).expect("backup yydx");
        conn.close();
        blob
    };

    let restored = Connection::restore_yydx(&backup).expect("restore yydx backup");
    assert_eq!(restored.get("assets/status").expect("read status"), Some(b"hot".to_vec()));
    assert_eq!(
        restored.get("assets/payload").expect("read attachment"),
        Some(format!("yydb:object:{}", blob.hash_hex()).into_bytes())
    );
    assert_eq!(&restored.get_object(&blob).expect("read backed-up blob")[..], payload.as_slice());
    restored.close();
}

// fixture: yydb.scenario.vos_transaction_backup
#[test]
fn yydb_scenario_vos_transaction_backup() {
    let dir = TempDb::new("scenario-vos-transaction-backup");
    let live_path = dir.yydx();
    let backup = backup_path(&live_path);

    {
        let conn = Connection::open_yydx(&live_path).expect("open yydx");
        conn.ensure_schema(USER_SCHEMA).expect("ensure schema");
        conn.begin().expect("begin transaction");
        conn.execute_vos_write(
            r#"User {
                user_id: "550e8400-e29b-41d4-a716-446655440000",
                user_name: "ada",
                active: true,
            }.insert()"#,
        )
        .expect("insert user inside the transaction");
        conn.commit().expect("commit transaction");

        let live_rows = conn.query_vos("User.all().collect()").expect("query live rows");
        assert_eq!(live_rows.len(), 1, "the committed VOS row must be visible");
        assert_eq!(live_rows[0].get("user_name"), Some(&Value::Text("ada".into())));
        conn.backup_yydx_to(&backup).expect("backup yydx");
        conn.close();
    }

    let restored = Connection::restore_yydx(&backup).expect("restore yydx backup");
    let rows = restored.query_vos("User.all().collect()").expect("query restored rows");
    assert_eq!(rows.len(), 1, "the committed VOS row must survive backup restore");
    assert_eq!(rows[0].get("user_name"), Some(&Value::Text("ada".into())));
    assert_eq!(rows[0].get("active"), Some(&Value::Bool(true)));
    assert_eq!(rows[0].get("user_id").map(Value::kind), Some("uuid"));
    restored.close();
}

// fixture: yydb.scenario.yydx_compact_backup
#[test]
fn yydb_scenario_yydx_compact_backup() {
    let dir = TempDb::new("scenario-yydx-compact-backup");
    let live_path = dir.yydx();
    let backup = backup_path(&live_path);
    let live_payload = large_payload(31);

    let live = {
        let conn = Connection::open_yydx(&live_path).expect("open yydx");
        let live = conn.put_chunk(ObjectKind::Blob, &live_payload).expect("store live blob");
        let mut attachment = Batch::new();
        attachment.attach_object(&live, "assets/live");
        attachment.commit(&conn).expect("commit live attachment");
        let first_orphan = conn.put_chunk(ObjectKind::Blob, b"orphan-one").expect("store orphan one");
        let second_orphan = conn.put_chunk(ObjectKind::Blob, b"orphan-two").expect("store orphan two");

        let report = conn.compact_yydx().expect("compact yydx");
        assert!(report.checkpointed, "compaction must fold the journal into the main file");
        assert_eq!(report.orphans_reclaimed, 2, "compaction must reclaim exactly the two unreferenced blobs");
        assert!(conn.scan_orphans("").expect("scan after compaction").is_empty());
        assert!(conn.get_object(&first_orphan).is_err(), "a reclaimed orphan must be gone");
        assert!(conn.get_object(&second_orphan).is_err(), "a reclaimed orphan must be gone");
        assert_eq!(&conn.get_object(&live).expect("live blob survives compaction")[..], live_payload.as_slice());

        conn.backup_yydx_to(&backup).expect("backup compacted yydx");
        conn.close();
        live
    };

    let restored = Connection::restore_yydx(&backup).expect("restore compacted yydx backup");
    assert_eq!(
        restored.get("assets/live").expect("read live attachment"),
        Some(format!("yydb:object:{}", live.hash_hex()).into_bytes())
    );
    assert_eq!(&restored.get_object(&live).expect("read backed-up live blob")[..], live_payload.as_slice());
    assert_eq!(restored.verify_object_references().expect("verify object closure"), 1);
    assert!(restored.scan_orphans("").expect("scan restored backup").is_empty());
    restored.close();
}
