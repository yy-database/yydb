//! `file` column lifecycle: descriptor shape, manifest validation, failure
//! paths and reference-driven reclamation.
//!
//! The host-coverable parts of this surface migrated to the binding suites:
//! manifest publishing with chunk counts, single-file embedding and backup,
//! descriptor identity through rename, reopen with full-content reread, window EOF
//! semantics, zero-octet files and tampered-chunk digest failures are covered
//! by `yydb.py`'s `test_yydb.py` and `@yydb/yydb`'s `file-channel` suite. The
//! tests below stay Rust-side because they assert internals no binding
//! exposes: the tagged descriptor and manifest wire shapes, missing-object
//! injection, reference counting, reclamation and backup/restore verification.

use std::io::Cursor;

use yydb::{ChunkManifest, Error, FileDescriptor, FileManifestRef, Value, types::scalar};

use crate::fixtures::yydb::{cleanup, open_temp_db, open_temp_yydx, reopen};

fn payload(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index % 251) as u8).collect()
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
fn single_file_yydb_embeds_files_and_restores_from_a_self_contained_backup() {
    let (conn, path) = open_temp_db("file-single-embed");

    conn.ensure_schema(ASSET_SCHEMA).unwrap();
    let octets = payload(3000);
    let file = conn.put_file(Cursor::new(octets.clone()), 500, Some("asset.bin".into()), None).unwrap();
    insert_asset(&conn, 1, &file).unwrap();
    assert_eq!(conn.read_file(&file).unwrap(), octets, "the embedded chunks must read back in-session");
    assert!(!crate::fixtures::yydb::objects_sidecar(&path).exists(), "single-file .yydb must not create a sibling object tree");

    let dest = crate::fixtures::yydb::temp_db_path("file-single-embed-backup");
    conn.backup_to(&dest).unwrap();
    drop(conn);

    let restored = yydb::Connection::restore_yydb(&dest).unwrap();
    assert!(restored.verify_object_references().unwrap() > 0, "restore must verify the embedded chunks");
    let rows = restored.query_vos("Asset.all().collect()").unwrap();
    let stored = match rows[0].get("payload") {
        Some(Value::File(stored)) => stored.clone(),
        other => panic!("restored payload must be a file descriptor, got {other:?}"),
    };
    assert_eq!(stored, file);
    assert_eq!(restored.read_file(&stored).unwrap(), octets, "the backup carries the embedded payloads");

    cleanup(&path);
    cleanup(&dest);
}

#[test]
fn single_file_yydb_survives_a_put_file_handle_being_dropped() {
    let (conn, path) = open_temp_db("file-single-handle");
    conn.ensure_schema(ASSET_SCHEMA).unwrap();
    let octets = payload(1200);
    let file = conn.put_file(Cursor::new(octets.clone()), 300, Some("asset.bin".into()), None).unwrap();
    drop(conn);

    // Hosts that open a handle per operation must see the object from the next
    // handle: the payload is already in the main image when `put_file` returns.
    let second = yydb::Connection::open(&path).unwrap();
    insert_asset(&second, 1, &file).unwrap();
    drop(second);

    let third = yydb::Connection::open(&path).unwrap();
    let rows = third.query_vos("Asset.all().collect()").unwrap();
    let stored = match rows[0].get("payload") {
        Some(Value::File(stored)) => stored.clone(),
        other => panic!("payload must be a file descriptor, got {other:?}"),
    };
    assert_eq!(stored, file);
    assert_eq!(third.read_file(&stored).unwrap(), octets, "a later handle must re-read the embedded chunks");
    assert!(!crate::fixtures::yydb::objects_sidecar(&path).exists(), "single-file .yydb must not create a sibling object tree");
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

    let restored = yydb::Connection::restore_yydx(&dest).unwrap();
    assert_eq!(restored.verify_file_digests().unwrap(), 1, "restore verified the one committed file value");
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
fn restore_fails_closed_on_missing_or_tampered_objects() {
    use yydb::ObjectStore;

    let (conn, path) = open_temp_yydx("file-restore-fail");
    conn.ensure_schema(ASSET_SCHEMA).unwrap();
    let octets = payload(700);
    let file = conn.put_file(Cursor::new(octets.clone()), 256, None, None).unwrap();
    insert_asset(&conn, 1, &file).unwrap();

    let dest = crate::fixtures::yydb::temp_yydx("file-restore-fail-dest");
    conn.backup_yydx_to(&dest).unwrap();
    drop(conn);

    let manifest = {
        let good = yydb::Connection::restore_yydx(&dest).expect("a complete backup restores cleanly");
        let manifest = good.objects().load_file_manifest(&file).unwrap();
        drop(good);
        manifest
    };

    let victim_path = ObjectStore::yydx_objects_root(&dest)
        .join(&victim_dir(&manifest.chunks[0]))
        .join(format!("{}.blob", manifest.chunks[0].hash_hex()));
    std::fs::remove_file(&victim_path).unwrap();
    let error = yydb::Connection::restore_yydx(&dest).err().unwrap();
    assert!(matches!(error, Error::ObjectNotFound { .. }), "a missing chunk must fail the restore: {error:?}");

    std::fs::write(&victim_path, b"swapped content").unwrap();
    let error = yydb::Connection::restore_yydx(&dest).err().unwrap();
    assert!(
        matches!(error, Error::ObjectCorrupt { .. }),
        "a chunk that does not match its manifest hash must fail the restore: {error:?}"
    );

    cleanup(&path);
    cleanup(&dest);
}

fn victim_dir(chunk: &yydb::ObjectRef) -> String {
    chunk.hash_hex().chars().take(2).collect()
}

const FILE_VECTOR_SCHEMA: &str = r#"
table Embedding {
    @@id: i64,
    payload: file,
    embedding: vector<121>,
}
"#;

fn insert_embedding(conn: &yydb::Connection, id: i64, file: &FileDescriptor, embedding: Value) -> yydb::Result<()> {
    conn.execute_vos_write_with_params(
        "Embedding::insert({ id: $id, payload: $payload, embedding: $embedding })",
        &std::collections::BTreeMap::from([
            ("id".to_string(), Value::I64(id)),
            ("payload".to_string(), Value::File(file.clone())),
            ("embedding".to_string(), embedding),
        ]),
    )
}

/// Hash of the `vector-payload` object the stored row references for `embedding`.
///
/// Vector placement is content-addressed, so the object the row points at is the
/// one CAS returns for the same components.
fn vector_object_hash(conn: &yydb::Connection, embedding: &[f32]) -> String {
    let vector = yydb::Vector::new(embedding.to_vec()).expect("vector");
    conn.objects().put_vector(&vector).expect("vector payload object").hash_hex()
}

#[test]
fn restore_verifies_every_object_reference_not_only_file_digests() {
    let (conn, path) = open_temp_yydx("file-vector-restore");
    conn.ensure_schema(FILE_VECTOR_SCHEMA).unwrap();
    let octets = payload(700);
    let file = conn.put_file(Cursor::new(octets.clone()), 256, Some("asset.bin".into()), None).unwrap();
    let embedding: Vec<f32> = (0..121).map(|index| index as f32 * 0.5 + 0.25).collect();
    insert_embedding(&conn, 1, &file, Value::Vector(yydb::Vector::new(embedding.clone()).unwrap())).unwrap();

    let vector_hash = vector_object_hash(&conn, &embedding);
    assert!(conn.objects().path_for_hash(&vector_hash).exists(), "the vector object reference must be stored as a blob");

    let dest = crate::fixtures::yydb::temp_yydx("file-vector-restore-dest");
    conn.backup_yydx_to(&dest).unwrap();
    drop(conn);

    let restored = yydb::Connection::restore_yydx(&dest).expect("a complete backup restores cleanly");
    assert_eq!(
        restored.verify_object_references().unwrap(),
        5,
        "the manifest, three chunks, and the vector payload are all referenced objects"
    );
    assert_eq!(restored.verify_file_digests().unwrap(), 1);
    let rows = restored.query_vos("Embedding.all().collect()").unwrap();
    let stored_file = match rows[0].get("payload") {
        Some(Value::File(stored)) => stored.clone(),
        other => panic!("restored payload must be a file descriptor, got {other:?}"),
    };
    assert_eq!(stored_file, file);
    assert_eq!(restored.read_file(&stored_file).unwrap(), octets);
    let stored_vector = match rows[0].get("embedding") {
        Some(Value::Vector(stored)) => stored,
        other => panic!("restored embedding must resolve to a vector, got {other:?}"),
    };
    assert_eq!(stored_vector.data.as_ref(), embedding.as_slice(), "the copied object tree must carry the vector payload");
    drop(restored);

    let copied = yydb::ObjectStore::yydx_objects_root(&dest).join(&vector_hash[0..2]).join(format!("{vector_hash}.blob"));
    std::fs::remove_file(&copied).unwrap();
    let error = yydb::Connection::restore_yydx(&dest).err().unwrap();
    assert!(matches!(error, Error::ObjectNotFound { .. }), "a missing vector payload must fail the restore: {error:?}");

    std::fs::write(&copied, b"swapped content").unwrap();
    let error = yydb::Connection::restore_yydx(&dest).err().unwrap();
    assert!(
        matches!(error, Error::ObjectCorrupt { .. }),
        "a vector payload that does not match its reference must fail the restore: {error:?}"
    );

    cleanup(&path);
    cleanup(&dest);
}

#[test]
fn doctor_reports_a_missing_manifest_and_reclaim_refuses() {
    let (conn, path) = open_temp_yydx("file-doctor-manifest");
    conn.ensure_schema(ASSET_SCHEMA).unwrap();
    let file = conn.put_file(Cursor::new(payload(700)), 256, None, None).unwrap();
    insert_asset(&conn, 1, &file).unwrap();

    let manifest_path = conn.objects().path_for(&file.manifest_object());
    let manifest_hex = file.manifest_object().hash_hex();
    drop(conn);
    std::fs::remove_file(&manifest_path).unwrap();

    let reopened = reopen(&path);
    let report = reopened.doctor().expect("doctor must report the gap instead of failing");
    assert!(
        report.issues.iter().any(|issue| issue.code == "yydb.doctor.missing_blob" && issue.message.contains(&manifest_hex)),
        "doctor must name the missing manifest, got {:?}",
        report.issues
    );
    let error = reopened.reclaim_orphans(&[]).unwrap_err();
    assert!(matches!(error, Error::ObjectNotFound { .. }), "reclaim must stay fail-closed: {error:?}");

    cleanup(&path);
}

#[test]
fn lifecycle_closed_loop_publish_backup_restore_reclaim() {
    let (conn, path) = open_temp_yydx("file-lifecycle");
    conn.ensure_schema(ASSET_SCHEMA).unwrap();
    let octets = payload(900);
    let file = conn.put_file(Cursor::new(octets.clone()), 256, Some("asset.bin".into()), None).unwrap();
    insert_asset(&conn, 1, &file).unwrap();

    assert!(conn.scan_orphans("").unwrap().is_empty(), "a committed row pins every object");

    let dest = crate::fixtures::yydb::temp_yydx("file-lifecycle-dest");
    conn.backup_yydx_to(&dest).unwrap();
    let restored = yydb::Connection::restore_yydx(&dest).expect("restore verifies the digest");
    assert_eq!(restored.read_file(&file).unwrap(), octets, "the backup reads back with the exact digest");

    delete_asset(&conn, 1).unwrap();
    let orphans = conn.scan_orphans("").unwrap();
    assert_eq!(orphans.len(), 5, "the row deletion releases the manifest plus the four 256-byte chunks");
    assert_eq!(conn.reclaim_orphans(&orphans).unwrap().reclaimed_objects, 5);
    let error = conn.read_file(&file).unwrap_err();
    assert!(matches!(error, Error::ObjectNotFound { .. }), "reclaimed content must not read: {error:?}");

    assert_eq!(restored.read_file(&file).unwrap(), octets, "the pre-deletion backup is unaffected");

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
