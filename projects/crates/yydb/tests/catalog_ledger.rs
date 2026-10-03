mod common;

use std::collections::BTreeMap;
use yydb::vos::ast::{FieldId, FieldPath, RenameMap};
use yydb::{Connection, Error, OpenFlags};

const INITIAL: &str = "table User { @@id: i64, name: utf8, active: bool }";
const REORDERED: &str = "table User { active: bool, name: utf8, @@id: i64 }";

#[test]
fn persists_reorder_rename_and_tombstones_across_reopen() {
    let (connection, path) = common::open_temp_db("catalog-ledger");
    connection.ensure_schema(INITIAL).unwrap();
    let initial = connection.catalog_snapshot().unwrap().unwrap();
    connection
        .migrate_schema(REORDERED, &RenameMap::default())
        .unwrap();
    drop(connection);
    let connection = common::reopen(&path);
    let reordered = connection.catalog_snapshot().unwrap().unwrap();
    assert_eq!(reordered.types[0].type_id, initial.types[0].type_id);
    for (before, after) in initial.types[0]
        .fields
        .iter()
        .zip(&reordered.types[0].fields)
    {
        assert_eq!(before.field_id, after.field_id);
        assert_eq!(before.virtual_field, after.virtual_field);
    }
    let renames = RenameMap {
        fields: BTreeMap::from([(
            FieldPath {
                type_name: "User".into(),
                field_name: "name".into(),
            },
            "label".into(),
        )]),
        ..RenameMap::default()
    };
    connection
        .migrate_schema("table User { @@id: i64, label: utf8 }", &renames)
        .unwrap();
    connection
        .migrate_schema(
            "table User { @@id: i64, label: utf8, status: bool }",
            &RenameMap::default(),
        )
        .unwrap();
    drop(connection);
    let connection = common::reopen(&path);
    let snapshot = connection.catalog_snapshot().unwrap().unwrap();
    let contract = connection.resolved_contract().unwrap().unwrap();
    assert_eq!(contract.types[0].type_id, snapshot.types[0].type_id.0);
    assert_eq!(
        contract.types[0].fields[1].field_id,
        snapshot.types[0].fields[1].field_id.0
    );
    assert_eq!(snapshot.types[0].fields[1].field_id, FieldId(2));
    assert_eq!(snapshot.types[0].fields[1].current_name, "label");
    assert_eq!(snapshot.retired_fields[0].field_id, FieldId(3));
    assert_eq!(snapshot.types[0].fields[2].field_id, FieldId(4));
    assert_eq!(snapshot.types[0].fields[2].virtual_field, 3);
    let execution = connection.execution_catalog().unwrap().unwrap();
    assert_eq!(execution.types[0].fields[2].handle.field_id(), 4);
    assert_eq!(execution.types[0].fields[2].handle.index(), 3);
    assert_eq!(connection.schema_version().unwrap(), Some(4));
    drop(connection);
    common::cleanup(&path);
}

#[test]
fn failed_migrations_leave_schema_and_ledger_unchanged() {
    let connection = Connection::open_in_memory().unwrap();
    connection.ensure_schema(INITIAL).unwrap();
    let initial = connection.catalog_snapshot().unwrap();
    assert!(matches!(
        connection.ensure_schema(REORDERED),
        Err(Error::Schema { .. })
    ));
    assert!(connection.ensure_schema(REORDERED).is_err());
    let empty = Connection::open_in_memory().unwrap();
    assert!(empty
        .migrate_schema(REORDERED, &RenameMap::default())
        .is_err());
    let bad_pragma = format!("// @yydb-schema-version: 9\n{REORDERED}");
    assert!(connection
        .migrate_schema(&bad_pragma, &RenameMap::default())
        .is_err());
    let invalid = RenameMap {
        types: BTreeMap::from([("Missing".into(), "User".into())]),
        ..RenameMap::default()
    };
    assert!(connection.migrate_schema(REORDERED, &invalid).is_err());
    assert_eq!(connection.schema().unwrap().unwrap().document, INITIAL);
    assert_eq!(connection.catalog_snapshot().unwrap(), initial);
    assert_eq!(connection.schema_version().unwrap(), Some(1));
}

#[test]
fn recovers_the_same_ledger_from_wal_before_checkpoint() {
    let (_, path) = common::open_temp_db("catalog-wal");
    let connection = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    connection.ensure_schema(INITIAL).unwrap();
    connection
        .migrate_schema(REORDERED, &RenameMap::default())
        .unwrap();
    let expected = connection.catalog_snapshot().unwrap();
    drop(connection);
    let connection = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    assert_eq!(connection.catalog_snapshot().unwrap(), expected);
    assert_eq!(connection.schema_version().unwrap(), Some(2));
    connection.checkpoint().unwrap();
    drop(connection);
    let connection = common::reopen(&path);
    assert_eq!(connection.catalog_snapshot().unwrap(), expected);
    drop(connection);
    common::cleanup(&path);
}

#[test]
fn legacy_source_requires_explicit_ledger_initialization() {
    let (connection, path) = common::open_temp_db("catalog-legacy");
    drop(connection);
    let mut legacy = b"YYDB\x01".to_vec();
    legacy.push(1);
    legacy.extend(1u32.to_le_bytes());
    legacy.extend((INITIAL.len() as u32).to_le_bytes());
    legacy.extend(INITIAL.as_bytes());
    legacy.extend(0u32.to_le_bytes());
    std::fs::write(&path, legacy).unwrap();
    let connection = common::reopen(&path);
    assert!(connection.catalog_snapshot().unwrap().is_none());
    connection.put("plain", b"value").unwrap();
    assert!(std::fs::read(&path).unwrap().starts_with(b"YYDB\x01"));
    assert!(connection
        .migrate_schema(REORDERED, &RenameMap::default())
        .is_err());
    connection.ensure_schema(INITIAL).unwrap();
    assert!(std::fs::read(&path).unwrap().starts_with(b"YYDB\x03"));
    drop(connection);
    let connection = common::reopen(&path);
    assert!(connection.catalog_snapshot().unwrap().is_some());
    assert_eq!(connection.get("plain").unwrap(), Some(b"value".to_vec()));
    drop(connection);
    common::cleanup(&path);
}

#[test]
fn upgrades_v2_catalog_without_reassigning_identity() {
    let (connection, path) = common::open_temp_db("catalog-v2-upgrade");
    connection.ensure_schema(INITIAL).unwrap();
    let expected = connection.catalog_snapshot().unwrap().unwrap();
    let resolved = connection.resolved_contract().unwrap().unwrap();
    drop(connection);

    let mut bytes = std::fs::read(&path).unwrap();
    let resolved_bytes = serde_json::to_vec(&resolved).unwrap();
    assert!(bytes.ends_with(&resolved_bytes));
    bytes.truncate(bytes.len() - resolved_bytes.len() - 5);
    bytes[4] = 2;
    std::fs::write(&path, bytes).unwrap();

    let connection = common::reopen(&path);
    assert!(connection.resolved_contract().unwrap().is_none());
    connection.ensure_schema(INITIAL).unwrap();
    assert_eq!(connection.catalog_snapshot().unwrap().unwrap(), expected);
    assert!(connection.resolved_contract().unwrap().is_some());
    assert!(std::fs::read(&path).unwrap().starts_with(b"YYDB\x03"));
    drop(connection);
    common::cleanup(&path);
}

#[test]
fn rejects_persisted_ledger_with_duplicate_identity() {
    let (connection, path) = common::open_temp_db("catalog-corrupt");
    connection.ensure_schema(INITIAL).unwrap();
    let mut snapshot = connection.catalog_snapshot().unwrap().unwrap();
    let contract = connection.resolved_contract().unwrap().unwrap();
    drop(connection);
    let mut bytes = std::fs::read(&path).unwrap();
    let original = serde_json::to_vec(&snapshot).unwrap();
    let resolved = serde_json::to_vec(&contract).unwrap();
    assert!(bytes.ends_with(&resolved));
    bytes.truncate(bytes.len() - resolved.len() - 5);
    assert!(bytes.ends_with(&original));
    bytes.truncate(bytes.len() - original.len() - 4);
    snapshot.types[0].fields[1].field_id = snapshot.types[0].fields[0].field_id;
    let malformed = serde_json::to_vec(&snapshot).unwrap();
    bytes.extend((malformed.len() as u32).to_le_bytes());
    bytes.extend(malformed);
    std::fs::write(&path, bytes).unwrap();
    assert!(matches!(Connection::open(&path), Err(Error::Corrupt(_))));
    common::cleanup(&path);
}

#[test]
fn ignores_an_incomplete_wal_tail_after_the_last_complete_snapshot() {
    let (_, path) = common::open_temp_db("wal-tail");
    let connection = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    connection.ensure_schema(INITIAL).unwrap();
    connection.put("value", b"complete").unwrap();
    drop(connection);
    let mut bytes = std::fs::read(yydb::journal::wal_path(&path)).unwrap();
    bytes.truncate(bytes.len() - 3);
    std::fs::write(yydb::journal::wal_path(&path), bytes).unwrap();
    let connection = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    assert_eq!(connection.schema_version().unwrap(), Some(1));
    assert_eq!(connection.get("value").unwrap(), None);
    drop(connection);
    common::cleanup(&path);
}

#[test]
fn rejects_a_complete_wal_frame_with_a_bad_checksum() {
    let (_, path) = common::open_temp_db("wal-checksum");
    let connection = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    connection.ensure_schema(INITIAL).unwrap();
    drop(connection);
    let wal = yydb::journal::wal_path(&path);
    let mut bytes = std::fs::read(&wal).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x80;
    std::fs::write(&wal, bytes).unwrap();
    assert!(matches!(
        Connection::open_with_flags(&path, OpenFlags::wal()),
        Err(Error::Corrupt(_))
    ));
    common::cleanup(&path);
}
