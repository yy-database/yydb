//! Round-trip tests of the SQLite format-3 writer against the crate's reader.
//!
//! External-implementation verification (Python `sqlite3`, `better-sqlite3`)
//! lives in the `@yydb/sqlite` TypeScript suite; these tests pin the writer's
//! own structural invariants: b-tree splits, overflow pages, freeblocks, the
//! freelist, index maintenance and transaction rollback.

use sqlite_formats::{WriteValue, read_freelist_count};
use sqlite_storage::{ColumnSpec, SchemaObjectKind, SqliteWriter, TableLimits, UpdateValue};

fn column(name: &str, declared: &str) -> ColumnSpec {
    ColumnSpec {
        name: name.to_string(),
        declared_type: declared.to_string(),
        not_null: false,
        default_value: None,
        primary_key: false,
        unique: false,
    }
}

fn text(value: &str) -> Option<WriteValue> {
    Some(WriteValue::Text(value.to_string()))
}

fn integer(value: i64) -> Option<WriteValue> {
    Some(WriteValue::Integer(value))
}

fn limits() -> TableLimits {
    TableLimits::default()
}

#[test]
fn create_table_and_insert_round_trip_through_the_reader() {
    let mut writer = SqliteWriter::create(4096).expect("create database");
    writer
        .create_table("items", &[column("id", "INTEGER"), column("name", "TEXT"), column("note", "TEXT")])
        .expect("create table");
    let first = writer.insert("items", &[integer(1), text("alpha"), None]).expect("insert first");
    let second = writer.insert("items", &[integer(2), text("中文"), text("")]).expect("insert second");
    assert_eq!((first, second), (1, 2));

    let rows = writer.table_rows("items", limits()).expect("read rows");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].rowid, 1);
    assert_eq!(rows[1].rowid, 2);

    let schema = writer.schema_objects(limits()).expect("read schema");
    assert!(schema.iter().any(|object| object.kind == SchemaObjectKind::Table && object.name == "items"));
}

#[test]
fn inserts_beyond_one_page_split_the_btree_and_stay_ordered() {
    // A 512-byte page holds only a handful of rows, so this forces splits and
    // interior pages.
    let mut writer = SqliteWriter::create(512).expect("create database");
    writer.create_table("bulk", &[column("id", "INTEGER"), column("label", "TEXT")]).expect("create table");
    for index in 1..=200i64 {
        writer
            .insert("bulk", &[integer(index), text(&format!("row-{index:03}"))])
            .unwrap_or_else(|error| panic!("insert {index}: {error}"));
    }
    let rows = writer.table_rows("bulk", limits()).expect("read rows");
    assert_eq!(rows.len(), 200);
    for (position, row) in rows.iter().enumerate() {
        assert_eq!(row.rowid, position as i64 + 1, "rows must stay in rowid order");
    }
}

#[test]
fn out_of_order_inserts_keep_interior_routing_intact() {
    // Sequential rowids only ever descend the rightmost path, so this
    // permutation forces splits of interior children that are NOT rightmost —
    // the case where the upper cell must keep the child's old maximum key.
    let mut writer = SqliteWriter::create(512).expect("create database");
    writer.create_table("shuffled", &[column("id", "INTEGER"), column("label", "TEXT")]).expect("create table");
    let mut order: Vec<i64> = (1..=300).collect();
    // Deterministic shuffle (LCG), independent of host RNG.
    let mut state = 0x2545F491u64;
    for index in (1..order.len()).rev() {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let swap = (state >> 33) as usize % (index + 1);
        order.swap(index, swap);
    }
    for rowid in order {
        writer
            .insert("shuffled", &[integer(rowid), text(&format!("row-{rowid}"))])
            .unwrap_or_else(|error| panic!("insert {rowid}: {error}"));
    }
    let rows = writer.table_rows("shuffled", limits()).expect("read rows");
    assert_eq!(rows.len(), 300, "no row may be lost or duplicated by splits");
    for (position, row) in rows.iter().enumerate() {
        assert_eq!(row.rowid, position as i64 + 1, "rows must stay in rowid order");
    }
}

#[test]
fn large_alternating_null_index_survives_splits() {
    // 500 rows with alternating NULL/REAL keys force the index tree through
    // several splits where the divider lands on a real entry; external SQLite
    // counts interior divider cells as entries, so the writer must move them
    // out of the leaves. The Rust-side invariant: every mutation still
    // round-trips through the reader and the unique check stays exact.
    let mut writer = SqliteWriter::create(4096).expect("create database");
    let mut id = column("id", "INTEGER");
    id.primary_key = true;
    writer.create_table("t", &[id, column("score", "REAL"), column("v", "TEXT")]).expect("create table");
    writer.create_index("t_score", "t", &["score".to_string()], false).expect("create index");
    for index in 1..=500i64 {
        let score = if index % 2 == 0 { Some(WriteValue::Real(index as f64 + 0.5)) } else { None };
        writer
            .insert("t", &[integer(index), score, text(&format!("v{index}"))])
            .unwrap_or_else(|error| panic!("insert {index}: {error}"));
    }
    writer.delete("t", 4).expect("delete");
    writer
        .update("t", 3, &[UpdateValue::Keep, UpdateValue::Keep, UpdateValue::Set(WriteValue::Text("updated".to_string()))])
        .expect("update");
    let rows = writer.table_rows("t", limits()).expect("read rows");
    assert_eq!(rows.len(), 499);
    assert!(rows.iter().all(|row| row.rowid != 4));
}

#[test]
fn five_hundred_sequential_inserts_never_duplicate_or_lose_rows() {
    // Pure-insert regression: sequential rowids at 4096-byte pages force leaf
    // and interior splits along the rightmost path only, while later inserts
    // land in the middle. Any routing overlap shows up as duplicate or lost
    // rowids here and as `integrity_check` failures for upstream SQLite.
    let mut writer = SqliteWriter::create(4096).expect("create database");
    let mut id = column("id", "INTEGER");
    id.primary_key = true;
    writer.create_table("seq", &[id, column("v", "TEXT")]).expect("create table");
    for index in 1..=500i64 {
        writer
            .insert("seq", &[integer(index), text(&format!("v{index}"))])
            .unwrap_or_else(|error| panic!("insert {index}: {error}"));
    }
    let rows = writer.table_rows("seq", limits()).expect("read rows");
    assert_eq!(rows.len(), 500, "no row may be lost or duplicated");
    for (position, row) in rows.iter().enumerate() {
        assert_eq!(row.rowid, position as i64 + 1);
    }
}

#[test]
fn overflow_cell_survives_leaf_splits() {
    let mut writer = SqliteWriter::create(4096).expect("create database");
    let mut id = column("id", "INTEGER");
    id.primary_key = true;
    writer.create_table("big", &[id, column("body", "TEXT")]).expect("create table");
    for index in 1..=300i64 {
        writer.insert("big", &[integer(index), text(&format!("v{index}"))]).expect("insert small");
    }
    let huge = "甲乙丙丁戊己庚辛壬癸-external-".repeat(1_200); // 48 KB, overflow cell
    writer.insert("big", &[integer(400), Some(WriteValue::Text(huge.clone()))]).expect("insert huge");
    writer.insert("big", &[integer(401), text("after")]).expect("insert after");

    let rows = writer.table_rows("big", limits()).expect("read rows");
    assert_eq!(rows.len(), 302, "the overflow row must survive splits");
    let huge_row = rows.iter().find(|row| row.rowid == 400).expect("rowid 400 present");
    let decoded = sqlite_storage::decode_record(&huge_row.payload, sqlite_storage::TextEncoding::Utf8, Default::default())
        .expect("decode huge row");
    match &decoded[1] {
        sqlite_formats::RecordValue::Text(value) => assert_eq!(value, &huge),
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn overflow_payloads_round_trip() {
    let mut writer = SqliteWriter::create(1024).expect("create database");
    writer.create_table("docs", &[column("id", "INTEGER"), column("body", "TEXT")]).expect("create table");
    let big_text = "甲乙丙DEF-".repeat(3_000); // ~39 KB, needs overflow pages
    writer.insert("docs", &[integer(1), Some(WriteValue::Text(big_text.clone()))]).expect("insert big text");
    writer.insert("docs", &[integer(2), text("small")]).expect("insert small text");

    let rows = writer.table_rows("docs", limits()).expect("read rows");
    assert_eq!(rows.len(), 2);
    let decoded = sqlite_storage::decode_record(&rows[0].payload, sqlite_storage::TextEncoding::Utf8, Default::default())
        .expect("decode big row");
    match &decoded[1] {
        sqlite_formats::RecordValue::Text(value) => assert_eq!(value, &big_text),
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn deletes_leave_freeblocks_and_freelist_pages() {
    let mut writer = SqliteWriter::create(512).expect("create database");
    writer.create_table("bulk", &[column("id", "INTEGER"), column("label", "TEXT")]).expect("create table");
    for index in 1..=120i64 {
        writer.insert("bulk", &[integer(index), text(&format!("row-{index}"))]).expect("insert");
    }
    // Scattered deletes create freeblocks; the contiguous tail delete should
    // empty whole pages and feed the freelist. Some tail rows already fell to
    // the scattered pass, so only surviving rows are asserted.
    let scattered: Vec<i64> = (1..=120i64).step_by(7).collect();
    for index in &scattered {
        assert!(writer.delete("bulk", *index).expect("delete scattered"));
    }
    for index in 61..=120i64 {
        if scattered.contains(&index) {
            continue;
        }
        assert!(writer.delete("bulk", index).expect("delete range"));
    }
    let rows = writer.table_rows("bulk", limits()).expect("read rows");
    let expected: Vec<i64> = (1..=60i64).filter(|index| !scattered.contains(index)).collect();
    assert_eq!(rows.iter().map(|row| row.rowid).collect::<Vec<_>>(), expected);
    assert!(read_freelist_count(writer.pages()) > 0, "deleted pages must reach the freelist");
    assert!(!writer.delete("bulk", 999_999).expect("missing delete returns false"));
}

#[test]
fn updates_rewrite_records_and_move_rowid_aliases() {
    let mut writer = SqliteWriter::create(4096).expect("create database");
    let mut id = column("id", "INTEGER");
    id.primary_key = true;
    writer.create_table("people", &[id, column("name", "TEXT")]).expect("create table");
    writer.insert("people", &[integer(10), text("before")]).expect("insert");

    let changed = writer
        .update("people", 10, &[UpdateValue::Keep, UpdateValue::Set(WriteValue::Text("after".to_string()))])
        .expect("update value");
    assert!(changed);
    let rows = writer.table_rows("people", limits()).expect("read rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].rowid, 10);

    let moved = writer
        .update("people", 10, &[UpdateValue::Set(WriteValue::Integer(42)), UpdateValue::Keep])
        .expect("update rowid alias");
    assert!(moved);
    let rows = writer.table_rows("people", limits()).expect("read rows after rowid move");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].rowid, 42);
}

#[test]
fn unique_constraints_reject_duplicates() {
    let mut writer = SqliteWriter::create(4096).expect("create database");
    let mut sku = column("sku", "TEXT");
    sku.unique = true;
    writer.create_table("products", &[sku, column("name", "TEXT")]).expect("create table");
    writer.insert("products", &[text("a-1"), text("first")]).expect("insert");
    let duplicate = writer.insert("products", &[text("a-1"), text("second")]);
    assert!(duplicate.is_err(), "duplicate UNIQUE value must be rejected");

    let mut id = column("id", "INTEGER");
    id.primary_key = true;
    writer.create_table("fixed", &[id, column("v", "TEXT")]).expect("create fixed");
    writer.insert("fixed", &[integer(7), text("x")]).expect("insert fixed");
    assert!(writer.insert("fixed", &[integer(7), text("y")]).is_err(), "duplicate rowid alias must be rejected");
}

#[test]
fn not_null_and_defaults_apply_on_insert() {
    let mut writer = SqliteWriter::create(4096).expect("create database");
    let mut name = column("name", "TEXT");
    name.not_null = true;
    name.default_value = Some(WriteValue::Text("anon".to_string()));
    let mut amount = column("amount", "REAL");
    amount.default_value = Some(WriteValue::Real(1.5));
    writer.create_table("rules", &[name, amount]).expect("create table");

    writer.insert("rules", &[None, None]).expect("defaults apply");
    let rows = writer.table_rows("rules", limits()).expect("read rows");
    let decoded = sqlite_storage::decode_record(&rows[0].payload, sqlite_storage::TextEncoding::Utf8, Default::default())
        .expect("decode");
    assert_eq!(decoded[0], sqlite_formats::RecordValue::Text("anon".to_string()));
    assert_eq!(decoded[1], sqlite_formats::RecordValue::Real(1.5));

    let rejected = writer.insert("rules", &[Some(WriteValue::Null), None]);
    assert!(rejected.is_err(), "explicit NULL into NOT NULL must fail even with a default");
}

#[test]
fn rollback_restores_the_image_and_commit_keeps_it() {
    let mut writer = SqliteWriter::create(4096).expect("create database");
    writer.create_table("txn", &[column("id", "INTEGER"), column("v", "TEXT")]).expect("create table");
    writer.insert("txn", &[integer(1), text("kept")]).expect("insert");

    writer.begin().expect("begin");
    writer.insert("txn", &[integer(2), text("rolled-back")]).expect("insert inside txn");
    writer.rollback().expect("rollback");
    let rows = writer.table_rows("txn", limits()).expect("read after rollback");
    assert_eq!(rows.len(), 1, "rolled-back insert must vanish");

    writer.begin().expect("begin again");
    writer.insert("txn", &[integer(3), text("committed")]).expect("insert inside txn");
    writer.commit().expect("commit");
    let rows = writer.table_rows("txn", limits()).expect("read after commit");
    assert_eq!(rows.len(), 2);
    assert!(writer.begin().is_ok());
    assert!(writer.begin().is_err(), "nested transactions are rejected");
    writer.rollback().expect("cleanup");
}

#[test]
fn indexes_backfill_and_maintain_on_mutation() {
    let mut writer = SqliteWriter::create(512).expect("create database");
    writer.create_table("users", &[column("id", "INTEGER"), column("email", "TEXT")]).expect("create table");
    for index in 1..=40i64 {
        writer.insert("users", &[integer(index), text(&format!("user{index}@example.com"))]).expect("insert");
    }
    writer.create_index("users_email", "users", &["email".to_string()], true).expect("create index");

    let schema = writer.schema_objects(limits()).expect("read schema");
    assert!(schema.iter().any(|object| object.kind == SchemaObjectKind::Index && object.name == "users_email"));

    // The unique index must reject duplicates after backfill.
    assert!(writer.insert("users", &[integer(41), text("user1@example.com")]).is_err());

    writer.delete("users", 1).expect("delete row 1");
    writer.insert("users", &[integer(42), text("user1@example.com")]).expect("reuse freed unique value");
    let rows = writer.table_rows("users", limits()).expect("read rows");
    assert_eq!(rows.len(), 40);
}

#[test]
fn index_deletes_emptying_leaves_keep_extract_max_safe() {
    // Regression: index deletes that empty whole leaf pages used to leave
    // husks whose later `extract_max` (deleting the divider entry above them)
    // underflowed with an index-out-of-bounds panic. Small pages force many
    // leaves; the interleaved update/delete churn empties some of them and
    // then deletes the divider entries routing to the husks. The crate-level
    // invariant: no panic, survivors stay readable and deletions stay gone.
    // External `integrity_check` evidence lives in the `@yydb/sqlite` suite.
    let mut writer = SqliteWriter::create(4096).expect("create database");
    let mut id = column("id", "INTEGER");
    id.primary_key = true;
    writer.create_table("t", &[id, column("a", "INTEGER"), column("b", "TEXT"), column("c", "REAL")]).expect("create table");
    writer.create_index("t_a", "t", &["a".to_string()], false).expect("create index a");
    writer.create_index("t_b", "t", &["b".to_string()], false).expect("create index b");
    writer.create_index("t_c", "t", &["c".to_string()], false).expect("create index c");
    let rows = 10_000i64;
    for index in 1..=rows {
        writer
            .insert(
                "t",
                &[
                    integer(index),
                    integer((index * 7919) % 50021),
                    text(&format!("grp-{}", (index * 131) % 977)),
                    Some(WriteValue::Real((index * 37 % 1000) as f64 / 4.0)),
                ],
            )
            .unwrap_or_else(|error| panic!("insert {index}: {error}"));
    }
    for n in 0..rows / 20 {
        let index = n * 613 % rows + 1;
        writer
            .update(
                "t",
                index,
                &[UpdateValue::Keep, UpdateValue::Set(WriteValue::Integer(index + 1)), UpdateValue::Keep, UpdateValue::Keep],
            )
            .expect("update");
    }
    let mut deleted = 0i64;
    for n in 0..rows / 20 {
        let index = n * 1777 % rows + 1;
        if writer.delete("t", index).expect("delete") {
            deleted += 1;
        }
    }
    let rows_read = writer.table_rows("t", limits()).expect("read rows");
    assert_eq!(rows_read.len() as i64, rows - deleted, "survivors stay readable");
    let removed: std::collections::BTreeSet<i64> = (0..rows / 20).map(|n| n * 1777 % rows + 1).collect();
    assert!(rows_read.iter().all(|row| !removed.contains(&row.rowid)), "deleted rows stay deleted");
}

#[test]
fn save_writes_a_validated_file_that_reopens() {
    let directory = tempfile_path("sqlite-writer-save");
    let mut writer = SqliteWriter::create(4096).expect("create database");
    writer.create_table("persist", &[column("id", "INTEGER"), column("v", "TEXT")]).expect("create table");
    writer.insert("persist", &[integer(1), text("durable")]).expect("insert");
    writer.save(&directory).expect("save");

    let reopened = SqliteWriter::open(&directory).expect("reopen");
    let rows = reopened.table_rows("persist", limits()).expect("read after reopen");
    assert_eq!(rows.len(), 1);
    std::fs::remove_file(&directory).ok();
}

fn tempfile_path(stem: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("clock").as_nanos();
    std::env::temp_dir().join(format!("{stem}-{nanos}.sqlite"))
}
