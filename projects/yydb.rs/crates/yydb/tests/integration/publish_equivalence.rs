//! Narrowed publication must equal a full rebuild on the same sequence.
//!
//! The full publish path scans every stored record and diffs it against the new
//! state. Narrowed publication instead lists the records one keyed statement
//! touched, so the two paths discover the same change set by completely
//! different means.
//!
//! This suite runs one operation sequence twice — the default narrowed path and
//! `set_full_publish`, which forces the scanning path for the same process —
//! and requires the images to agree row by row, object by object, before and
//! after a reopen. Both journal modes are covered because narrowed publication
//! reaches the pager through two different commit paths.

use std::collections::BTreeMap;

use yydb::{Connection, Error, INLINE_BYTES_MAX, Value, journal::OpenFlags, query::QueryRow};

use crate::fixtures::yydb::{cleanup, reopen, temp_db};

const SCHEMA: &str = r#"
table Doc {
    @@doc_id: i64,
    title: utf8,
}

table Blob {
    @@blob_id: i64,
    label: utf8,
    body: bytes,
}

table Keyed {
    @@key: utf8,
    @code: utf8,
    n: i64,
}
"#;

fn octets(length: usize) -> Vec<u8> {
    (0..length).map(|index| if index % 5 == 0 { 0x00 } else { 0xff }).collect()
}

fn insert_doc(conn: &Connection, doc_id: i64, title: &str) {
    conn.execute_vos_write(&format!(r#"Doc::insert({{ doc_id: {doc_id}, title: "{title}" }})"#)).unwrap();
}

fn insert_blob(conn: &Connection, blob_id: i64, body: Vec<u8>) {
    conn.execute_vos_write_with_params(
        r#"Blob::insert({ blob_id: $id, label: $label, body: $body })"#,
        &BTreeMap::from([
            ("id".to_string(), Value::I64(blob_id)),
            ("label".to_string(), Value::Text(format!("row-{blob_id}"))),
            ("body".to_string(), Value::Bytes(body)),
        ]),
    )
    .unwrap();
}

fn insert_keyed(conn: &Connection, key: &str, code: &str, count: i64) {
    conn.execute_vos_write(&format!(r#"Keyed::insert({{ key: "{key}", code: "{code}", n: {count} }})"#)).unwrap();
}

/// One operation sequence that reaches every publication branch.
///
/// It mixes narrowed writes (keyed inserts, patches and deletes on rule-free
/// tables), a rule-carrying table that must keep the scanning path, embedded
/// object records on both sides of the inline boundary, rejected writes that
/// must leave nothing behind, and a rolled-back transaction.
fn apply_sequence(conn: &Connection) {
    conn.ensure_schema(SCHEMA).unwrap();

    insert_doc(conn, 1, "one");
    insert_doc(conn, 2, "two");
    insert_doc(conn, 3, "three");
    insert_doc(conn, 1, "one replaced");
    conn.execute_vos_write(r#"Doc::update({ doc_id: 2, title: "two patched" })"#).unwrap();
    conn.execute_vos_write(r#"Doc::delete({ doc_id: 3 })"#).unwrap();
    conn.execute_vos_write(r#"Doc::delete({ doc_id: 99 })"#).unwrap();

    insert_blob(conn, 1, octets(INLINE_BYTES_MAX + 1));
    insert_blob(conn, 2, octets(INLINE_BYTES_MAX));
    insert_blob(conn, 1, octets(INLINE_BYTES_MAX * 2));
    conn.execute_vos_write(r#"Blob::update({ blob_id: 2, label: "boundary patched" })"#).unwrap();

    insert_keyed(conn, "a", "A", 1);
    insert_keyed(conn, "b", "B", 2);
    let error = conn.execute_vos_write(r#"Keyed::insert({ key: "c", code: "A", n: 3 })"#).expect_err("duplicate code");
    assert!(matches!(error, Error::UniqueViolation { .. }), "unexpected error: {error}");

    let error = conn.execute_vos_write(r#"Doc::update({ title: "no key" })"#).expect_err("missing primary key");
    assert!(error.to_string().contains("`doc_id`"), "unexpected error: {error}");

    conn.begin().unwrap();
    conn.execute_vos_write(r#"Doc::update({ doc_id: 1, title: "never committed" })"#).unwrap();
    conn.rollback().unwrap();
}

/// The same publication branches driven through the SQL write surface.
///
/// The benchmark harness writes through SQL, so this sequence proves the SQL
/// frontend narrows to the same result as a full rebuild instead of relying on
/// the VOS planner alone.
fn apply_sql_sequence(conn: &Connection) {
    conn.ensure_schema(SCHEMA).unwrap();
    let insert_doc_sql = "INSERT INTO Doc (doc_id, title) VALUES (?1, ?2)";
    for (doc_id, title) in [(1, "one"), (2, "two"), (3, "three"), (1, "one replaced")] {
        conn.execute_sql_write(insert_doc_sql, &[Value::I64(doc_id), Value::Text(title.into())]).unwrap();
    }
    conn.execute_sql_write(
        "INSERT INTO Doc (doc_id, title) VALUES (?1, ?2), (?3, ?4)",
        &[Value::I64(4), Value::Text("four".into()), Value::I64(5), Value::Text("five".into())],
    )
    .unwrap();
    conn.execute_sql_write("UPDATE Doc SET title = ?1 WHERE doc_id = ?2", &[Value::Text("two patched".into()), Value::I64(2)])
        .unwrap();
    conn.execute_sql_write("DELETE FROM Doc WHERE doc_id = ?1", &[Value::I64(3)]).unwrap();
    conn.execute_sql_write("DELETE FROM Doc WHERE doc_id = ?1", &[Value::I64(99)]).unwrap();
    // A numeric key range cannot map onto record-key order, so this one keeps
    // the scanning path and must still publish correctly.
    conn.execute_sql_write("DELETE FROM Doc WHERE doc_id >= ?1", &[Value::I64(4)]).unwrap();

    let blob_sql = "INSERT INTO Blob (blob_id, label, body) VALUES (?1, ?2, ?3)";
    conn.execute_sql_write(blob_sql, &[Value::I64(1), Value::Text("row-1".into()), Value::Bytes(octets(INLINE_BYTES_MAX + 1))])
        .unwrap();
    conn.execute_sql_write(blob_sql, &[Value::I64(2), Value::Text("row-2".into()), Value::Bytes(octets(INLINE_BYTES_MAX))])
        .unwrap();
    conn.execute_sql_write(blob_sql, &[Value::I64(1), Value::Text("row-1".into()), Value::Bytes(octets(INLINE_BYTES_MAX * 2))])
        .unwrap();
    conn.execute_sql_write(
        "UPDATE Blob SET label = ?1 WHERE blob_id = ?2",
        &[Value::Text("boundary patched".into()), Value::I64(2)],
    )
    .unwrap();

    // `key` is a reserved word for the SQL frontend, so the rule-carrying table
    // is reached through VOS. Both planners must still refuse to narrow it.
    insert_keyed(conn, "a", "A", 1);
    let error = conn.execute_vos_write(r#"Keyed::insert({ key: "b", code: "A", n: 2 })"#).expect_err("duplicate code");
    assert!(matches!(error, Error::UniqueViolation { .. }), "unexpected error: {error}");

    conn.begin().unwrap();
    conn.execute_sql_write(
        "UPDATE Doc SET title = ?1 WHERE doc_id = ?2",
        &[Value::Text("never committed".into()), Value::I64(1)],
    )
    .unwrap();
    conn.rollback().unwrap();
}

fn rows(conn: &Connection, table: &str) -> Vec<QueryRow> {
    conn.query_vos(&format!("{table}.all().collect()")).unwrap()
}

fn assert_equivalent(narrowed: &Connection, full: &Connection, stage: &str) {
    for table in ["Doc", "Blob", "Keyed"] {
        assert_eq!(rows(narrowed, table), rows(full, table), "{stage}: `{table}` rows must be identical");
    }
    let mut narrowed_objects: Vec<String> =
        narrowed.objects().list_objects().unwrap().iter().map(|object| object.hash_hex()).collect();
    let mut full_objects: Vec<String> = full.objects().list_objects().unwrap().iter().map(|object| object.hash_hex()).collect();
    narrowed_objects.sort();
    full_objects.sort();
    assert_eq!(narrowed_objects, full_objects, "{stage}: both images must carry the same object set");
}

fn run_pair(label: &str, flags: OpenFlags) {
    run_pair_with(label, flags, apply_sequence);
}

fn run_pair_with(label: &str, flags: OpenFlags, sequence: fn(&Connection)) {
    let narrowed_path = temp_db(&format!("{label}-narrowed"));
    let full_path = temp_db(&format!("{label}-full"));
    let narrowed = Connection::open_with_flags(&narrowed_path, flags).unwrap();
    let full = Connection::open_with_flags(&full_path, flags).unwrap();
    full.set_full_publish(true);

    sequence(&narrowed);
    sequence(&full);
    assert_equivalent(&narrowed, &full, "before reopen");

    narrowed.close();
    drop(narrowed);
    full.close();
    drop(full);

    let narrowed = reopen(&narrowed_path);
    let full = reopen(&full_path);
    assert_equivalent(&narrowed, &full, "after reopen");
    narrowed.close();
    full.close();
    cleanup(&narrowed_path);
    cleanup(&full_path);
}

#[test]
fn narrowed_publication_matches_a_full_rebuild_with_delete_journal() {
    run_pair("publish-equivalence-delete", OpenFlags::new());
}

#[test]
fn narrowed_publication_matches_a_full_rebuild_under_wal() {
    run_pair("publish-equivalence-wal", OpenFlags::wal());
}

#[test]
fn narrowed_publication_matches_a_full_rebuild_for_sql_writes() {
    run_pair_with("publish-equivalence-sql", OpenFlags::wal(), apply_sql_sequence);
}

/// A narrowed write on one table must leave every other row byte-identical.
///
/// The equivalence pair above only proves the two publication paths agree.
/// This case fixes the stronger property the narrowing relies on: rows outside
/// the statement's read window are neither rewritten nor dropped.
#[test]
fn narrowed_delete_and_patch_leave_unrelated_rows_untouched() {
    let path = temp_db("publish-narrowed-blast-radius");
    let conn = Connection::open(&path).unwrap();
    conn.ensure_schema(SCHEMA).unwrap();
    for doc_id in 1..=200 {
        insert_doc(&conn, doc_id, &format!("row-{doc_id}"));
    }
    let before: Vec<QueryRow> = rows(&conn, "Doc");
    assert_eq!(before.len(), 200);

    conn.execute_vos_write(r#"Doc::update({ doc_id: 7, title: "seven patched" })"#).unwrap();
    conn.execute_vos_write(r#"Doc::delete({ doc_id: 150 })"#).unwrap();

    let after: Vec<QueryRow> = rows(&conn, "Doc");
    assert_eq!(after.len(), 199);
    let survivors: Vec<&QueryRow> = before.iter().filter(|row| row.get("doc_id") != Some(&Value::I64(150))).collect();
    assert_eq!(survivors.len(), 199);
    for (before_row, after_row) in survivors.iter().zip(after.iter()) {
        assert_eq!(before_row.get("doc_id"), after_row.get("doc_id"), "row order must follow the primary key");
        if before_row.get("doc_id") == Some(&Value::I64(7)) {
            assert_ne!(before_row.get("title"), after_row.get("title"), "the patched row must change");
        }
        else {
            assert_eq!(**before_row, *after_row, "a keyed write must not disturb row {:?}", before_row.get("doc_id"));
        }
    }

    conn.close();
    drop(conn);
    let reopened = reopen(&path);
    assert_eq!(rows(&reopened, "Doc").len(), 199, "the narrowed delete must be durable");
    reopened.close();
    cleanup(&path);
}

/// A transaction batch: every statement after `begin` reaches the in-place
/// transaction path instead of cloning the whole state per write.
fn apply_transaction_batch(conn: &Connection) {
    conn.ensure_schema(SCHEMA).unwrap();
    for chunk_start in (0..600).step_by(200) {
        conn.begin().unwrap();
        for doc_id in chunk_start..chunk_start + 200 {
            insert_doc(conn, doc_id, &format!("row-{doc_id}"));
        }
        conn.commit().unwrap();
    }
    conn.begin().unwrap();
    conn.execute_vos_write(r#"Doc::update({ doc_id: 7, title: "patched in a transaction" })"#).unwrap();
    conn.execute_vos_write(r#"Doc::delete({ doc_id: 42 })"#).unwrap();
    insert_keyed(conn, "txn", "TXN", 1);
    conn.commit().unwrap();
    conn.begin().unwrap();
    conn.execute_vos_write(r#"Doc::delete({ doc_id: 7 })"#).unwrap();
    conn.rollback().unwrap();
}

#[test]
fn a_transaction_batch_matches_a_full_rebuild() {
    run_pair_with("publish-equivalence-txn-batch", OpenFlags::wal(), apply_transaction_batch);
}

/// A transaction that creates embedded objects must publish the object rows it
/// added, not just the user rows.
fn apply_transaction_bytes(conn: &Connection) {
    conn.ensure_schema(SCHEMA).unwrap();
    conn.begin().unwrap();
    insert_blob(conn, 1, octets(INLINE_BYTES_MAX + 1));
    insert_blob(conn, 2, octets(INLINE_BYTES_MAX * 2));
    insert_doc(conn, 1, "one");
    conn.commit().unwrap();
    // The second transaction replaces one payload, so its delta has to carry
    // newly embedded object rows on top of the patched user row.
    conn.begin().unwrap();
    insert_blob(conn, 1, octets(INLINE_BYTES_MAX * 4));
    conn.commit().unwrap();
}

#[test]
fn a_transaction_that_embeds_objects_matches_a_full_rebuild() {
    run_pair_with("publish-equivalence-txn-bytes", OpenFlags::wal(), apply_transaction_bytes);
}

/// A savepoint restores the whole transaction buffer, so the commit that
/// follows it must publish the restored state, not only the rows written
/// after the savepoint.
fn apply_savepoint_sequence(conn: &Connection) {
    conn.ensure_schema(SCHEMA).unwrap();
    insert_doc(conn, 1, "one");
    conn.begin().unwrap();
    insert_doc(conn, 2, "two");
    conn.savepoint("before_three").unwrap();
    insert_doc(conn, 3, "three");
    conn.execute_vos_write(r#"Doc::delete({ doc_id: 1 })"#).unwrap();
    conn.rollback_to_savepoint("before_three").unwrap();
    insert_doc(conn, 4, "four");
    conn.commit().unwrap();
}

#[test]
fn a_savepoint_rollback_commit_matches_a_full_rebuild() {
    run_pair_with("publish-equivalence-savepoint", OpenFlags::wal(), apply_savepoint_sequence);
}

/// A key deleted and recreated inside one transaction must publish its final
/// bytes, and a key created and deleted inside one transaction must vanish from
/// the image even though it never existed before.
fn apply_delete_then_insert(conn: &Connection) {
    conn.ensure_schema(SCHEMA).unwrap();
    insert_doc(conn, 1, "one");
    conn.begin().unwrap();
    conn.execute_vos_write(r#"Doc::delete({ doc_id: 1 })"#).unwrap();
    insert_doc(conn, 1, "one again");
    insert_doc(conn, 2, "two then gone");
    conn.execute_vos_write(r#"Doc::delete({ doc_id: 2 })"#).unwrap();
    conn.commit().unwrap();
}

#[test]
fn a_delete_then_insert_commit_matches_a_full_rebuild() {
    run_pair_with("publish-equivalence-delete-insert", OpenFlags::wal(), apply_delete_then_insert);
}

/// A write the planner cannot narrow marks the whole transaction wholesale, so
/// its commit must still publish every changed row it holds.
fn apply_unique_rule_transaction(conn: &Connection) {
    conn.ensure_schema(SCHEMA).unwrap();
    for doc_id in 1..=20 {
        insert_doc(conn, doc_id, &format!("row-{doc_id}"));
    }
    insert_keyed(conn, "a", "A", 1);
    conn.begin().unwrap();
    conn.execute_vos_write(r#"Doc::update({ doc_id: 9, title: "patched" })"#).unwrap();
    // `Keyed` carries a unique rule, so this statement cannot name its windows.
    insert_keyed(conn, "b", "B", 2);
    conn.commit().unwrap();
}

#[test]
fn a_unique_rule_write_makes_commit_publish_the_whole_transaction() {
    run_pair_with("publish-equivalence-unique-rule", OpenFlags::wal(), apply_unique_rule_transaction);

    let path = temp_db("publish-equivalence-unique-rule-durable");
    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    apply_unique_rule_transaction(&conn);
    conn.close();
    drop(conn);
    let reopened = reopen(&path);
    assert_eq!(rows(&reopened, "Keyed").len(), 2, "both rule-carrying rows must survive the commit");
    let docs = rows(&reopened, "Doc");
    assert_eq!(docs.len(), 20, "the narrowed row and its neighbours must all be durable");
    reopened.close();
    cleanup(&path);
}

/// A failing statement must undo itself inside the windows it touched and
/// leave the rest of the transaction exactly as it was.
#[test]
fn a_failed_statement_inside_a_transaction_leaves_earlier_writes_intact() {
    let path = temp_db("publish-equivalence-txn-failure");
    let conn = Connection::open(&path).unwrap();
    conn.ensure_schema(SCHEMA).unwrap();
    conn.begin().unwrap();
    insert_doc(&conn, 1, "one");
    insert_keyed(&conn, "a", "A", 1);
    let error = conn.execute_vos_write(r#"Keyed::insert({ key: "b", code: "A", n: 2 })"#).expect_err("duplicate code");
    assert!(matches!(error, Error::UniqueViolation { .. }), "unexpected error: {error}");
    insert_doc(&conn, 2, "two");
    conn.commit().unwrap();

    let docs = rows(&conn, "Doc");
    assert_eq!(docs.len(), 2, "the two successful inserts must survive the rejected one");
    assert_eq!(rows(&conn, "Keyed").len(), 1, "the rejected insert must leave no row behind");

    conn.close();
    drop(conn);
    let reopened = reopen(&path);
    assert_eq!(rows(&reopened, "Doc"), docs);
    assert_eq!(rows(&reopened, "Keyed").len(), 1);
    reopened.close();
    cleanup(&path);
}
