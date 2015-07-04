//! `file` column lifecycle: descriptor shape, manifest publishing, read windows,
//! and reference-driven reclamation.

use std::io::Cursor;

use yydb::{ChunkManifest, Error, FileDescriptor, FileManifestRef, ObjectKind, Value, types::scalar};

use crate::fixtures::yydb::{cleanup, open_temp_db, open_temp_yydx, reopen};

fn payload(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index % 251) as u8).collect()
}

#[test]
fn publishes_chunks_then_a_file_manifest() {
    let (conn, path) = open_temp_yydx("file-publish");
    let octets = payload(1000);
    let file = conn
        .put_file(Cursor::new(octets.clone()), 256, Some("asset.bin".into()), Some("application/octet-stream".into()))
        .unwrap();

    assert_eq!(file.octets, octets.len() as u64);
    assert_eq!(file.digest, yydb::vos::types::FileManifestIdentity::from_octets(&octets).digest);
    assert_eq!(file.name.as_deref(), Some("asset.bin"));
    assert_eq!(file.media_type.as_deref(), Some("application/octet-stream"));

    let manifest_object = file.manifest_object();
    assert_eq!(manifest_object.kind, ObjectKind::FileManifest);
    assert!(conn.objects().path_for(&manifest_object).exists());
    let manifest = ChunkManifest::from_payload_bytes(&conn.get_object(&manifest_object).unwrap()).unwrap();
    assert_eq!(manifest.total_size, octets.len() as u64);
    assert_eq!(manifest.chunks.len(), 4);
    assert_eq!(manifest.chunk_size, 256);
    for chunk in &manifest.chunks {
        assert_eq!(chunk.kind, ObjectKind::Blob);
        assert!(conn.objects().path_for(chunk).exists());
    }

    cleanup(&path);
}

#[test]
fn rejects_file_values_on_a_single_file_yydb() {
    let (conn, path) = open_temp_db("file-single-file");
    let error = conn.put_file(Cursor::new(payload(32)), 0, None, None).unwrap_err();
    assert!(matches!(error, Error::Unsupported(_)), "single-file `.yydb` must reject `file` values: {error:?}");
    cleanup(&path);
}

#[test]
fn file_descriptor_tagged_json_roundtrips_and_rejects_malformed_members() {
    let file = FileDescriptor {
        digest: "0f".repeat(32),
        octets: 17,
        name: Some("report.txt".into()),
        media_type: None,
        manifest: FileManifestRef { hash: [0x22; 32], size: 40 },
    };
    let value = Value::File(file.clone());
    let json = scalar::tagged_value_json(&value).expect("file values have a tagged form");
    assert_eq!(json["$yydb"], "file");
    assert_eq!(json["digest"], "0f".repeat(32));
    assert_eq!(json["octets"], 17);
    assert_eq!(json["name"], "report.txt");
    assert!(json.get("mediaType").is_none());
    assert_eq!(json["manifest"]["hash"], "22".repeat(32));
    assert_eq!(scalar::tagged_value_from_json(&json).expect("tagged object").unwrap(), value);

    let mut uppercase = json.clone();
    uppercase["digest"] = serde_json::Value::from("0F".repeat(32));
    assert!(scalar::tagged_value_from_json(&uppercase).expect("tagged object").is_err());

    let mut unknown = json.clone();
    unknown["extra"] = serde_json::Value::from(true);
    assert!(scalar::tagged_value_from_json(&unknown).expect("tagged object").is_err());

    let mut missing_manifest = json.clone();
    missing_manifest.as_object_mut().unwrap().remove("manifest");
    assert!(scalar::tagged_value_from_json(&missing_manifest).expect("tagged object").is_err());

    let mut short_digest = json.clone();
    short_digest["digest"] = serde_json::Value::from("0f".repeat(31));
    assert!(scalar::tagged_value_from_json(&short_digest).expect("tagged object").is_err());

    let mut extra_manifest = json.clone();
    extra_manifest["manifest"]["kind"] = serde_json::Value::from("file-manifest");
    assert!(scalar::tagged_value_from_json(&extra_manifest).expect("tagged object").is_err());

    let mut text_octets = json.clone();
    text_octets["octets"] = serde_json::Value::from("17");
    assert!(scalar::tagged_value_from_json(&text_octets).expect("tagged object").is_err());
}

#[test]
fn manifest_payload_rejects_unknown_members_and_size_mismatch() {
    let valid = serde_json::json!({
        "chunk_size": 4,
        "total_size": 6,
        "chunks": [
            { "hash": "11".repeat(32), "size": 4 },
            { "hash": "22".repeat(32), "size": 2 },
        ],
    });
    let manifest = ChunkManifest::from_payload_bytes(valid.to_string().as_bytes()).unwrap();
    assert_eq!(manifest.chunks.len(), 2);

    let mut unknown = valid.clone();
    unknown["extra"] = serde_json::Value::from(1);
    assert!(ChunkManifest::from_payload_bytes(unknown.to_string().as_bytes()).is_err());

    let mut mismatched = valid.clone();
    mismatched["total_size"] = serde_json::Value::from(7);
    assert!(ChunkManifest::from_payload_bytes(mismatched.to_string().as_bytes()).is_err());

    let mut short_middle = valid.clone();
    short_middle["chunks"][0]["size"] = serde_json::Value::from(2);
    assert!(ChunkManifest::from_payload_bytes(short_middle.to_string().as_bytes()).is_err());
}

#[test]
fn reopen_keeps_the_descriptor_and_manifest_objects() {
    let (conn, path) = open_temp_yydx("file-reopen");
    let octets = payload(700);
    let file = conn.put_file(Cursor::new(octets.clone()), 256, None, None).unwrap();
    drop(conn);

    let reopened = reopen(&path);
    let manifest = ChunkManifest::from_payload_bytes(&reopened.get_object(&file.manifest_object()).unwrap()).unwrap();
    assert_eq!(manifest.total_size, octets.len() as u64);
    assert!(reopened.objects().path_for(&file.manifest_object()).exists());
    assert_eq!(reopened.read_file(&file).unwrap(), octets, "reopen relocates and re-checks every chunk");
    cleanup(&path);
}

#[test]
fn reads_windows_with_vos_eof_semantics() {
    use yydb::vos::types::FileReadWindow;

    let (conn, path) = open_temp_yydx("file-windows");
    let octets = payload(1000);
    let file = conn.put_file(Cursor::new(octets.clone()), 256, None, None).unwrap();

    let read = |offset: u64, length: u64| conn.read_file_window(&file, FileReadWindow::new(offset, length)).unwrap();

    assert_eq!(read(0, 1000), octets);
    assert_eq!(read(0, 100), octets[..100]);
    assert_eq!(read(200, 200), octets[200..400]);
    assert_eq!(read(256, 256), octets[256..512]);
    assert_eq!(read(900, 500), octets[900..]);

    assert!(read(0, 0).is_empty());
    assert!(read(1000, 10).is_empty(), "an offset at end of file is an empty window");
    assert!(read(1500, 10).is_empty(), "an offset past end of file is an empty window");

    let manifest = conn.objects().load_file_manifest(&file).unwrap();
    assert_eq!(conn.read_file_range(&manifest, 200, 200).unwrap(), octets[200..400]);
    assert!(conn.read_file_range(&manifest, 2000, 100).unwrap().is_empty());
    cleanup(&path);
}

#[test]
fn reads_zero_octet_files_and_exact_chunk_multiples() {
    use yydb::vos::types::FileReadWindow;

    let (conn, path) = open_temp_yydx("file-boundaries");

    let empty = conn.put_file(Cursor::new(Vec::new()), 256, None, None).unwrap();
    assert_eq!(empty.octets, 0);
    assert_eq!(empty.digest, yydb::vos::types::FileManifestIdentity::from_octets(&[]).digest);
    assert!(conn.read_file(&empty).unwrap().is_empty());
    assert!(conn.read_file_window(&empty, FileReadWindow::new(0, 8)).unwrap().is_empty());
    assert!(conn.read_file_window(&empty, FileReadWindow::new(4, 8)).unwrap().is_empty());

    let octets = payload(512);
    let exact = conn.put_file(Cursor::new(octets.clone()), 256, None, None).unwrap();
    assert_eq!(conn.read_file(&exact).unwrap(), octets);
    assert_eq!(
        conn.read_file_window(&exact, FileReadWindow::new(256, 256)).unwrap(),
        octets[256..],
        "a window starting exactly at a chunk boundary reads the following chunk"
    );
    assert!(conn.read_file_window(&exact, FileReadWindow::new(512, 16)).unwrap().is_empty());

    let short = payload(100);
    let single = conn.put_file(Cursor::new(short.clone()), 256, None, None).unwrap();
    assert_eq!(conn.objects().load_file_manifest(&single).unwrap().chunks.len(), 1);
    assert_eq!(conn.read_file(&single).unwrap(), short);
    assert_eq!(conn.read_file_window(&single, FileReadWindow::new(40, 200)).unwrap(), short[40..]);
    cleanup(&path);
}

#[test]
fn reads_the_complete_file_and_verifies_the_digest() {
    let (conn, path) = open_temp_yydx("file-digest");
    let octets = payload(1000);
    let file = conn.put_file(Cursor::new(octets.clone()), 256, None, None).unwrap();
    assert_eq!(conn.read_file(&file).unwrap(), octets);

    let mut wrong = file.clone();
    wrong.digest = "0f".repeat(32);
    let error = conn.read_file(&wrong).unwrap_err();
    assert!(matches!(error, Error::ObjectCorrupt { .. }), "a rewritten digest must fail: {error:?}");
    cleanup(&path);
}

#[test]
fn reports_missing_objects_instead_of_empty_content() {
    use yydb::vos::types::FileReadWindow;

    let (conn, path) = open_temp_yydx("file-missing");
    let file = conn.put_file(Cursor::new(payload(300)), 256, None, None).unwrap();

    conn.objects().remove_object(&file.manifest_object()).unwrap();
    let error = conn.read_file_window(&file, FileReadWindow::new(0, 10)).unwrap_err();
    assert!(matches!(error, Error::ObjectNotFound { .. }), "a missing manifest must not read as empty: {error:?}");

    // Republishing the same octets restores the manifest, so removing one chunk
    // then exercises the missing-chunk path instead of the missing-manifest one.
    let file = conn.put_file(Cursor::new(payload(300)), 256, None, None).unwrap();
    let manifest = conn.objects().load_file_manifest(&file).unwrap();
    conn.objects().remove_object(&manifest.chunks[0]).unwrap();
    let error = conn.read_file_window(&file, FileReadWindow::new(0, 10)).unwrap_err();
    assert!(matches!(error, Error::ObjectNotFound { .. }), "a missing chunk must not read as empty: {error:?}");
    cleanup(&path);
}

#[test]
fn detects_a_tampered_chunk_after_reopen() {
    use yydb::vos::types::FileReadWindow;

    let (conn, path) = open_temp_yydx("file-tamper");
    let octets = payload(600);
    let file = conn.put_file(Cursor::new(octets), 256, None, None).unwrap();
    drop(conn);

    let reopened = reopen(&path);
    let manifest = reopened.objects().load_file_manifest(&file).unwrap();
    let chunk = manifest.chunks[0].clone();
    let chunk_path = reopened.objects().path_for(&chunk);
    let mut tampered = std::fs::read(&chunk_path).unwrap();
    tampered[0] ^= 0xff;
    std::fs::write(&chunk_path, &tampered).unwrap();

    let error = reopened.read_file_window(&file, FileReadWindow::new(0, 10)).unwrap_err();
    assert!(matches!(error, Error::ObjectCorrupt { .. }), "a tampered chunk must fail its hash check: {error:?}");
    cleanup(&path);
}

const ASSET_SCHEMA: &str = r#"
table Asset {
    @@asset_id: i64,
    payload: file,
}
"#;

const INSERT_ASSET: &str = "Asset::insert({ asset_id: $id, payload: $payload })";

fn insert_asset(conn: &yydb::Connection, id: i64, file: &FileDescriptor) -> yydb::Result<()> {
    conn.execute_vos_write_with_params(
        INSERT_ASSET,
        &std::collections::BTreeMap::from([
            ("id".to_string(), Value::I64(id)),
            ("payload".to_string(), Value::File(file.clone())),
        ]),
    )
}

fn delete_asset(conn: &yydb::Connection, id: i64) -> yydb::Result<usize> {
    conn.execute_sql_write("DELETE FROM Asset WHERE asset_id = ?", &[Value::I64(id)])
}

fn file_objects(conn: &yydb::Connection, file: &FileDescriptor) -> Vec<yydb::ObjectRef> {
    let manifest = conn.objects().load_file_manifest(file).unwrap();
    let mut objects = vec![file.manifest_object()];
    objects.extend(manifest.chunks);
    objects
}

#[test]
fn row_references_keep_objects_until_the_row_is_deleted() {
    use yydb::vos::types::FileReadWindow;

    let (conn, path) = open_temp_yydx("file-row-refs");
    conn.ensure_schema(ASSET_SCHEMA).unwrap();
    let octets = payload(700);
    let file = conn.put_file(Cursor::new(octets), 256, Some("kept.bin".into()), None).unwrap();
    insert_asset(&conn, 1, &file).unwrap();

    assert!(conn.scan_orphans("").unwrap().is_empty(), "a committed row pins the manifest and its chunks");
    assert_eq!(file_objects(&conn, &file).len(), 4, "one manifest plus three chunks");

    delete_asset(&conn, 1).unwrap();
    let orphans = conn.scan_orphans("").unwrap();
    assert_eq!(orphans.len(), 4, "deleting the row only releases the reference");
    assert_eq!(conn.reclaim_orphans(&orphans).unwrap().reclaimed_objects, 4);

    let error = conn.read_file_window(&file, FileReadWindow::new(0, 10)).unwrap_err();
    assert!(matches!(error, Error::ObjectNotFound { .. }), "reclaimed content must not read as empty: {error:?}");
    cleanup(&path);
}

#[test]
fn shared_digest_is_reclaimed_only_after_the_last_reference() {
    let (conn, path) = open_temp_yydx("file-shared");
    conn.ensure_schema(ASSET_SCHEMA).unwrap();
    let octets = payload(700);
    let first =
        conn.put_file(Cursor::new(octets.clone()), 256, Some("a.bin".into()), Some("application/octet-stream".into())).unwrap();
    let renamed = conn.put_file(Cursor::new(octets.clone()), 256, Some("b.bin".into()), None).unwrap();

    assert_eq!(first.digest, renamed.digest, "metadata never changes content identity");
    assert_eq!(first.manifest, renamed.manifest, "renaming reuses the same manifest object");
    insert_asset(&conn, 1, &first).unwrap();
    insert_asset(&conn, 2, &renamed).unwrap();

    delete_asset(&conn, 1).unwrap();
    assert!(conn.scan_orphans("").unwrap().is_empty(), "the remaining row still pins the shared content");
    let objects = file_objects(&conn, &first);
    assert_eq!(conn.reclaim_orphans(&objects).unwrap().reclaimed_objects, 0);
    assert_eq!(conn.read_file(&renamed).unwrap(), octets);

    delete_asset(&conn, 2).unwrap();
    assert_eq!(conn.reclaim_orphans(&objects).unwrap().reclaimed_objects, objects.len() as u64);
    cleanup(&path);
}

#[test]
fn rollback_leaves_file_objects_for_reclaim() {
    let (conn, path) = open_temp_yydx("file-rollback");
    conn.ensure_schema(ASSET_SCHEMA).unwrap();
    let file = conn.put_file(Cursor::new(payload(700)), 256, None, None).unwrap();

    conn.begin().unwrap();
    insert_asset(&conn, 1, &file).unwrap();
    conn.rollback().unwrap();

    assert!(conn.query_vos("Asset.all().collect()").unwrap().is_empty(), "a rolled back row is not published");
    let orphans = conn.scan_orphans("").unwrap();
    assert_eq!(orphans.len(), 4, "a rollback leaves objects instead of deleting shared content");
    assert_eq!(conn.reclaim_orphans(&orphans).unwrap().reclaimed_objects, 4);
    cleanup(&path);
}

#[test]
fn backup_and_restore_read_the_file_through_the_copied_object_tree() {
    let (conn, path) = open_temp_yydx("file-backup");
    conn.ensure_schema(ASSET_SCHEMA).unwrap();
    let octets = payload(900);
    let file = conn.put_file(Cursor::new(octets.clone()), 256, Some("asset.bin".into()), None).unwrap();
    insert_asset(&conn, 1, &file).unwrap();

    let dest = crate::fixtures::yydb::temp_yydx("file-backup-dest");
    conn.backup_yydx_to(&dest).unwrap();
    drop(conn);

    let restored = yydb::Connection::open_yydx(&dest).unwrap();
    let rows = restored.query_vos("Asset.all().collect()").unwrap();
    let stored = match rows[0].get("payload") {
        Some(Value::File(stored)) => stored.clone(),
        other => panic!("restored payload must be a file descriptor, got {other:?}"),
    };
    assert_eq!(stored, file);
    assert_eq!(restored.read_file(&stored).unwrap(), octets, "restore must copy the object tree, not just the main file");

    cleanup(&path);
    cleanup(&dest);
}

#[test]
fn doctor_reports_missing_blob_for_file_chunks() {
    let (conn, path) = open_temp_yydx("file-doctor");
    conn.ensure_schema(ASSET_SCHEMA).unwrap();
    let octets = payload(700);
    let file = conn.put_file(Cursor::new(octets), 256, None, None).unwrap();
    insert_asset(&conn, 1, &file).unwrap();

    let doctor_before = conn.doctor().unwrap();
    assert!(
        !doctor_before.issues.iter().any(|i| i.code == "yydb.doctor.missing_blob"),
        "clean database must not report missing blobs, got {:?}",
        doctor_before.issues.iter().map(|i| &i.code).collect::<Vec<_>>()
    );

    // Delete one chunk blob from disk — the manifest still lists it and the row
    // still pins the manifest, so an orphan scan wouldn't notice the gap.
    let manifest = conn.objects().load_file_manifest(&file).unwrap();
    let victim = manifest.chunks[0].clone();
    let victim_path = conn.objects().path_for(&victim);
    assert!(victim_path.exists(), "chunk must exist before deletion");
    drop(conn);
    std::fs::remove_file(&victim_path).unwrap();

    let reopened = reopen(&path);
    let doctor_after = reopened.doctor().unwrap();
    assert!(
        doctor_after.issues.iter().any(|i| i.code == "yydb.doctor.missing_blob"),
        "doctor must report missing chunk, got {:?}",
        doctor_after.issues
    );

    cleanup(&path);
}
