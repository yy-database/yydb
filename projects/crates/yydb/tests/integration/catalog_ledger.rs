use std::collections::BTreeMap;

use yydb::catalog::{FieldPath, RenameMap};
use yydb::{Connection, Error, OpenFlags};

const INITIAL: &str = "table User { @@id: i64, name: utf8, active: bool }";
const REORDERED: &str = "table User { active: bool, name: utf8, @@id: i64 }";

#[test]
fn persists_reorder_rename_and_tombstones_across_reopen() {
    let (connection, path) = crate::fixtures::yydb::open_temp_db("catalog-ledger");
    connection.ensure_schema(INITIAL).unwrap();
    let initial = connection.identity_history().unwrap().unwrap();
    connection
        .migrate_schema(REORDERED, &RenameMap::default())
        .unwrap();
    drop(connection);
    let connection = crate::fixtures::yydb::reopen(&path);
    let reordered = connection.identity_history().unwrap().unwrap();
    assert_eq!(
        reordered.snapshot.types[0].type_id,
        initial.snapshot.types[0].type_id
    );
    for before in &initial.snapshot.types[0].fields {
        let after = reordered.snapshot.types[0]
            .fields
            .iter()
            .find(|field| field.canonical_name == before.canonical_name)
            .expect("reordered field keeps its name");
        assert_eq!(before.field_id, after.field_id);
        assert_eq!(before.virtual_field_index, after.virtual_field_index);
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
    let connection = crate::fixtures::yydb::reopen(&path);
    let snapshot = connection.identity_history().unwrap().unwrap();
    let contract = connection.resolved_contract().unwrap().unwrap();
    assert_eq!(
        contract.types[0].type_id,
        snapshot.snapshot.types[0].type_id
    );
    let label = snapshot.snapshot.types[0]
        .fields
        .iter()
        .find(|field| field.canonical_name == "label")
        .expect("renamed field survives");
    assert_eq!(label.field_id, 2);
    assert_eq!(label.virtual_field_index, Some(1));
    let status = snapshot.snapshot.types[0]
        .fields
        .iter()
        .find(|field| field.canonical_name == "status")
        .expect("added field survives");
    assert_eq!(status.field_id, 4);
    assert_eq!(status.virtual_field_index, Some(3));
    assert_eq!(snapshot.retired_fields[0].field_id, 3);
    let execution = connection.execution_catalog().unwrap().unwrap();
    assert_eq!(execution.types[0].fields[2].handle.field_id(), 4);
    assert_eq!(execution.types[0].fields[2].handle.index(), 3);
    assert_eq!(connection.schema_version().unwrap(), Some(4));
    drop(connection);
    crate::fixtures::yydb::cleanup(&path);
}

#[test]
fn failed_migrations_leave_schema_and_ledger_unchanged() {
    let connection = Connection::open_in_memory().unwrap();
    connection.ensure_schema(INITIAL).unwrap();
    let initial = connection.identity_history().unwrap();
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
    assert_eq!(connection.identity_history().unwrap(), initial);
    assert_eq!(connection.schema_version().unwrap(), Some(1));
}

#[test]
fn recovers_the_same_ledger_from_wal_before_checkpoint() {
    let path = crate::fixtures::yydb::temp_db_path("catalog-wal");
    let connection = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    connection.ensure_schema(INITIAL).unwrap();
    connection
        .migrate_schema(REORDERED, &RenameMap::default())
        .unwrap();
    let expected = connection.identity_history().unwrap();
    drop(connection);
    let connection = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    assert_eq!(connection.identity_history().unwrap(), expected);
    assert_eq!(connection.schema_version().unwrap(), Some(2));
    connection.checkpoint().unwrap();
    drop(connection);
    let connection = crate::fixtures::yydb::reopen(&path);
    assert_eq!(connection.identity_history().unwrap(), expected);
    drop(connection);
    crate::fixtures::yydb::cleanup(&path);
}

#[test]
fn rejects_whole_state_database_magic() {
    let path = crate::fixtures::yydb::temp_db_path("reject-whole-state");
    std::fs::write(&path, b"YYDB\x01").unwrap();
    assert!(matches!(Connection::open(&path), Err(Error::Corrupt(_))));
    crate::fixtures::yydb::cleanup(&path);
}

#[test]
fn rejects_an_incomplete_wal_tail() {
    let path = crate::fixtures::yydb::temp_db_path("wal-tail");
    let connection = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    connection.ensure_schema(INITIAL).unwrap();
    connection.put("value", b"complete").unwrap();
    drop(connection);
    let mut bytes = std::fs::read(yydb::journal::wal_path(&path)).unwrap();
    bytes.truncate(bytes.len() - 3);
    std::fs::write(yydb::journal::wal_path(&path), bytes).unwrap();
    assert!(matches!(
        Connection::open_with_flags(&path, OpenFlags::wal()),
        Err(Error::Corrupt(_))
    ));
    crate::fixtures::yydb::cleanup(&path);
}

#[test]
fn rejects_a_complete_wal_frame_with_a_bad_checksum() {
    let path = crate::fixtures::yydb::temp_db_path("wal-checksum");
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
    crate::fixtures::yydb::cleanup(&path);
}
