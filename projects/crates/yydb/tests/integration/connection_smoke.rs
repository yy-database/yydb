use std::fs;

use crate::fixtures::yydb::{cleanup, objects_sidecar, temp_db, temp_yydx};

use yydb::{
    journal::{shm_path, wal_path},
    Connection, Error, JournalMode, ObjectKind, ObjectStore, OpenFlags, Tier, Value, Vector,
    INLINE_BYTES_MAX,
};

#[test]
fn stores_schema_and_records_in_one_reopenable_file() {
    let path = temp_db("reopen");
    let conn = Connection::open(&path).unwrap();
    conn.ensure_schema("table Project { @@id: uuid }").unwrap();
    conn.put("project/meta", b"Spark").unwrap();
    drop(conn);

    let reopened = Connection::open(&path).unwrap();
    assert_eq!(reopened.schema().unwrap().unwrap().version, 1);
    assert_eq!(reopened.record_count().unwrap(), 1);
    assert_eq!(
        reopened.get("project/meta").unwrap(),
        Some(b"Spark".to_vec())
    );
    cleanup(&path);
}

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
    assert_eq!(
        reopened.get("project/meta").unwrap(),
        Some(b"Spark".to_vec())
    );
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
fn batch_compare_exchange_and_prefix_scan_are_atomic_on_connection() {
    let conn = Connection::open_in_memory().unwrap();
    conn.write_batch(&[
        ("frontier/a".into(), Some(b"queued".to_vec())),
        ("frontier/b".into(), Some(b"queued".to_vec())),
        ("other/c".into(), Some(b"ignored".to_vec())),
    ])
    .unwrap();
    assert_eq!(conn.scan_prefix("frontier/", None, 10).unwrap().len(), 2);
    assert!(conn
        .compare_exchange("frontier/a", Some(b"queued"), Some(b"claimed"))
        .unwrap());
    assert!(!conn
        .compare_exchange("frontier/a", Some(b"queued"), Some(b"stale"))
        .unwrap());
    assert_eq!(conn.get("frontier/a").unwrap(), Some(b"claimed".to_vec()));
}

#[test]
fn rust_scalar_udf_create_and_call() {
    let conn = Connection::open_in_memory().unwrap();
    conn.create_scalar("double", 1, |args| match args {
        [Value::I64(n)] => Ok(Value::I64(n * 2)),
        _ => Err(yydb::Error::Udf {
            name: "double".into(),
            message: "expected i64".into(),
        }),
    })
    .unwrap();
    assert_eq!(
        conn.call_scalar("double", &[Value::I64(21)]).unwrap(),
        Value::I64(42)
    );
    assert!(matches!(
        conn.call_scalar("double", &[]),
        Err(yydb::Error::UdfArity {
            expected: 1,
            got: 0,
            ..
        })
    ));
    assert_eq!(conn.list_scalars(), vec!["double".to_owned()]);
    conn.remove_scalar("double").unwrap();
    assert!(matches!(
        conn.call_scalar("double", &[Value::I64(1)]),
        Err(yydb::Error::UdfNotFound { .. })
    ));
}

#[test]
fn wal_and_shm_sidecars_recover_without_checkpoint() {
    let path = temp_db("wal");
    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    assert_eq!(conn.journal_mode(), JournalMode::Wal);
    assert!(conn
        .wal_path()
        .unwrap()
        .to_string_lossy()
        .ends_with(".yydb-wal"));
    assert!(conn
        .shm_path()
        .unwrap()
        .to_string_lossy()
        .ends_with(".yydb-shm"));
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
fn open_yydb_rejects_chunked_files() {
    let path = temp_db("no-chunk");
    let conn = Connection::open(&path).unwrap();
    let payload = vec![0_u8; 2000];
    let err = conn
        .put_file_chunked(std::io::Cursor::new(payload), 500)
        .unwrap_err();
    assert!(matches!(err, Error::Unsupported(_)));
    cleanup(&path);
}

#[test]
fn open_yydx_uses_objects_root_and_allows_chunked_files() {
    let path = temp_yydx("layout");
    let blob_root = ObjectStore::yydx_objects_root(&path);
    let conn = Connection::open_yydx(&path).unwrap();
    let payload: Vec<u8> = (0..2000u16).map(|v| (v % 256) as u8).collect();
    let manifest = conn
        .put_file_chunked(std::io::Cursor::new(payload.clone()), 500)
        .unwrap();
    assert!(manifest.chunks.len() >= 4);
    assert!(blob_root.is_dir());
    assert!(
        conn.objects()
            .path_for(&manifest.chunks[0])
            .extension()
            .and_then(|ext| ext.to_str())
            == Some("blob")
    );
    assert!(!objects_sidecar(&path).exists());
    cleanup(&path);
}

#[test]
fn open_yydb_rejects_large_cas_payloads() {
    let path = temp_db("no-large-cas");
    let conn = Connection::open(&path).unwrap();
    let payload = vec![7_u8; INLINE_BYTES_MAX + 1];
    let err = conn.put_chunk(ObjectKind::Blob, &payload).unwrap_err();
    assert!(matches!(err, Error::Unsupported(_)));
    cleanup(&path);
}

#[test]
fn open_in_memory_does_not_create_temp_object_dirs() {
    let temp = std::env::temp_dir();
    let count_yydb_mem_dirs = || {
        std::fs::read_dir(&temp)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("yydb-mem-objects-")
            })
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
    let manifest = conn
        .put_file_chunked(std::io::Cursor::new(payload.clone()), 500)
        .unwrap();
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