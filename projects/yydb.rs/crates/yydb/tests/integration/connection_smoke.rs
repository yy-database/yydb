use std::fs;

use crate::fixtures::yydb::{cleanup, objects_sidecar, temp_db, temp_yydx};

use yydb::{
    Connection, Error, INLINE_BYTES_MAX, JournalMode, ObjectKind, ObjectStore, OpenFlags, Tier, Value, Vector,
    journal::{shm_path, wal_path},
};

// Connection-level KV reopen and batch/CAS smoke migrated to the TypeScript
// binding suite (`statement-txn-kv` in `@yydb/yydb`), which covers KV
// roundtrip/reopen, batch, compare-exchange and prefix scans through the
// N-API surface. The tests below stay Rust-side because they assert internal
// layout and recovery details no binding exposes: temp-file hygiene of the
// atomic publish, journal-mode switching with sidecar cleanup, object-store
// layout/CAS paths and chunk-window reads at the manifest level.

#[test]
fn atomically_replaces_main_image_without_leaving_temp_files() {
    let path = temp_db("atomic");
    let conn = Connection::open(&path).unwrap();
    conn.ensure_schema("table Project { @@id: uuid }").unwrap();
    conn.put("project/meta", b"Spark").unwrap();
    drop(conn);

    let file_name = path.file_name().unwrap().to_string_lossy();
    let prefix = format!(".{file_name}.yydb-tmp-");
    let leftovers = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .count();
    assert_eq!(leftovers, 0);

    let reopened = Connection::open(&path).unwrap();
    assert_eq!(reopened.get("project/meta").unwrap(), Some(b"Spark".to_vec()));
    drop(reopened);
    cleanup(&path);
}

#[test]
fn open_in_memory_roundtrip() {
    let conn = Connection::open_in_memory().unwrap();
    assert!(conn.path().is_none());
    conn.ensure_schema("table Demo { @@id: uuid }").unwrap();
    conn.put("k", b"v").unwrap();
    assert_eq!(conn.get("k").unwrap(), Some(b"v".to_vec()));
    assert_eq!(conn.schema().unwrap().unwrap().version, 1);
    assert_eq!(conn.record_count().unwrap(), 1);
}

#[test]
fn rust_scalar_udf_create_and_call() {
    let conn = Connection::open_in_memory().unwrap();
    conn.create_scalar("double", 1, |args| match args {
        [Value::I64(n)] => Ok(Value::I64(n * 2)),
        _ => Err(yydb::Error::Udf { name: "double".into(), message: "expected i64".into() }),
    })
    .unwrap();
    assert_eq!(conn.call_scalar("double", &[Value::I64(21)]).unwrap(), Value::I64(42));
    assert!(matches!(conn.call_scalar("double", &[]), Err(yydb::Error::UdfArity { expected: 1, got: 0, .. })));
    assert_eq!(conn.list_scalars(), vec!["double".to_owned()]);
    conn.remove_scalar("double").unwrap();
    assert!(matches!(conn.call_scalar("double", &[Value::I64(1)]), Err(yydb::Error::UdfNotFound { .. })));
}

#[test]
fn wal_and_shm_sidecars_recover_without_checkpoint() {
    let path = temp_db("wal");
    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    assert_eq!(conn.journal_mode(), JournalMode::Wal);
    assert!(conn.wal_path().unwrap().to_string_lossy().ends_with(".yydb-wal"));
    assert!(conn.shm_path().unwrap().to_string_lossy().ends_with(".yydb-shm"));
    conn.ensure_schema("table T { @@id: uuid }").unwrap();
    conn.put("a", b"1").unwrap();
    conn.put("b", b"2").unwrap();
    assert!(conn.wal_frame_count().unwrap() >= 2);
    assert!(wal_path(&path).exists());
    assert!(shm_path(&path).exists());
    drop(conn);

    let reopened = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    assert_eq!(reopened.get("a").unwrap(), Some(b"1".to_vec()));
    assert_eq!(reopened.get("b").unwrap(), Some(b"2".to_vec()));
    reopened.checkpoint().unwrap();
    assert_eq!(reopened.wal_frame_count().unwrap(), 0);
    drop(reopened);

    let after_ckpt = Connection::open(&path).unwrap();
    assert_eq!(after_ckpt.get("b").unwrap(), Some(b"2".to_vec()));
    cleanup(&path);
}

#[test]
fn set_journal_mode_wal_to_delete_removes_sidecars() {
    let path = temp_db("mode");
    let conn = Connection::open(&path).unwrap();
    conn.set_journal_mode(JournalMode::Wal).unwrap();
    conn.put("k", b"v").unwrap();
    assert!(wal_path(&path).exists());
    conn.set_journal_mode(JournalMode::Delete).unwrap();
    assert!(!wal_path(&path).exists());
    assert!(!shm_path(&path).exists());
    assert_eq!(conn.get("k").unwrap(), Some(b"v".to_vec()));
    cleanup(&path);
}

#[test]
fn open_yydb_does_not_create_objects_sidecar() {
    let path = temp_db("no-sidecar");
    let conn = Connection::open(&path).unwrap();
    let object = conn.put_chunk(ObjectKind::Blob, b"small").unwrap();
    assert_eq!(&*conn.get_object(&object).unwrap(), b"small");
    assert!(!objects_sidecar(&path).exists());
    cleanup(&path);
}

#[test]
fn open_yydb_embeds_chunked_files() {
    let path = temp_db("chunk-embed");
    let conn = Connection::open(&path).unwrap();
    let payload: Vec<u8> = (0..2000u16).map(|v| (v % 256) as u8).collect();
    let manifest = conn.put_file_chunked(std::io::Cursor::new(payload.clone()), 500).unwrap();
    assert!(manifest.chunks.len() >= 4);
    assert_eq!(conn.read_file_range(&manifest, 0, payload.len()).unwrap(), payload);
    assert!(!objects_sidecar(&path).exists());
    cleanup(&path);
}

#[test]
fn open_yydx_uses_objects_root_and_allows_chunked_files() {
    let path = temp_yydx("layout");
    let blob_root = ObjectStore::yydx_objects_root(&path);
    let conn = Connection::open_yydx(&path).unwrap();
    let payload: Vec<u8> = (0..2000u16).map(|v| (v % 256) as u8).collect();
    let manifest = conn.put_file_chunked(std::io::Cursor::new(payload.clone()), 500).unwrap();
    assert!(manifest.chunks.len() >= 4);
    assert!(blob_root.is_dir());
    assert!(conn.objects().path_for(&manifest.chunks[0]).extension().and_then(|ext| ext.to_str()) == Some("blob"));
    assert!(!objects_sidecar(&path).exists());
    cleanup(&path);
}

#[test]
fn open_yydb_embeds_large_cas_payloads_and_restores_them() {
    let path = temp_db("large-cas-embed");
    let backup = temp_db("large-cas-embed-backup");
    let conn = Connection::open(&path).unwrap();
    let payload = vec![7_u8; INLINE_BYTES_MAX + 1];
    let object = conn.put_chunk(ObjectKind::Blob, &payload).unwrap();
    assert_eq!(&*conn.get_object(&object).unwrap(), payload.as_slice());
    assert!(!objects_sidecar(&path).exists());
    conn.ensure_schema("table T {\n    @@t_id: i64,\n    value: i64,\n}").unwrap();

    conn.backup_to(&backup).unwrap();
    drop(conn);

    let restored = Connection::restore_yydb(&backup).unwrap();
    assert_eq!(&*restored.get_object(&object).unwrap(), payload.as_slice(), "backup must carry the embedded object");
    cleanup(&path);
    cleanup(&backup);
}

#[test]
fn open_in_memory_does_not_create_temp_object_dirs() {
    let temp = std::env::temp_dir();
    let count_yydb_mem_dirs = || {
        std::fs::read_dir(&temp)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("yydb-mem-objects-"))
            .count()
    };
    let before = count_yydb_mem_dirs();
    let conn = Connection::open_in_memory().unwrap();
    let object = conn.put_chunk(ObjectKind::Blob, b"ephemeral-only").unwrap();
    assert_eq!(&*conn.get_object(&object).unwrap(), b"ephemeral-only");
    assert_eq!(count_yydb_mem_dirs(), before);
}

#[test]
fn cas_objects_hash2_path_and_hot_cold() {
    let conn = Connection::open_in_memory().unwrap();
    let object = conn.put_chunk(ObjectKind::Blob, b"hello-cas").unwrap();
    let path = conn.objects().path_for(&object);
    let path_s = path.to_string_lossy().replace('\\', "/");
    assert!(path_s.ends_with(".blob"));
    let hex = object.hash_hex();
    assert!(path_s.contains(&format!("/{}/{}", &hex[..2], hex)));
    assert_eq!(&*conn.get_object(&object).unwrap(), b"hello-cas");
    assert_eq!(conn.pin_object(&object).unwrap(), Tier::Hot);
    assert_eq!(conn.object_tier(&object), Tier::Hot);
    assert_eq!(conn.evict_object(&object).unwrap(), Tier::Cold);
    assert_eq!(&*conn.get_object(&object).unwrap(), b"hello-cas");
}

#[test]
fn chunked_file_range_read() {
    let conn = Connection::open_in_memory().unwrap();
    let payload: Vec<u8> = (0..2000u16).map(|v| (v % 256) as u8).collect();
    let manifest = conn.put_file_chunked(std::io::Cursor::new(payload.clone()), 500).unwrap();
    assert!(manifest.chunks.len() >= 4);
    assert_eq!(manifest.total_size, 2000);
    let mid = conn.read_file_range(&manifest, 500, 500).unwrap();
    assert_eq!(mid, payload[500..1000]);
    let tail = conn.read_file_range(&manifest, 1800, 500).unwrap();
    assert_eq!(tail, payload[1800..]);
}

#[test]
fn vector_via_cas_roundtrip() {
    let conn = Connection::open_in_memory().unwrap();
    let vector = Vector::new(vec![1.0, 2.5, -3.0]).unwrap();
    let object = conn.put_vector(&vector).unwrap();
    assert_eq!(object.kind, ObjectKind::VectorPayload);
    let loaded = conn.get_vector(&object).unwrap();
    assert_eq!(loaded.dim, 3);
    assert_eq!(&*loaded.data, &*vector.data);
}
