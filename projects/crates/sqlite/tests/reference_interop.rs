//! Cross-checks the read-only reader against files produced by upstream SQLite.
//!
//! Fixtures are generated at test time through Python's bundled `sqlite3`
//! module. The reader result and the upstream `sqlite_master` /
//! `PRAGMA table_info` result are compared row by row and column by column.

mod common;

use sqlite::{
    RecordLimits, RecordValue, SchemaObjectKind, TableLimits, TextEncoding, decode_record, read_named_table, read_schema,
    validate_database,
};

use common::ReferenceValue;

fn kind_name(kind: SchemaObjectKind) -> &'static str {
    match kind {
        SchemaObjectKind::Table => "table",
        SchemaObjectKind::Index => "index",
        SchemaObjectKind::View => "view",
        SchemaObjectKind::Trigger => "trigger",
    }
}

#[test]
fn schema_matches_upstream_sqlite_for_both_page_sizes() {
    for page_size in [1024, 4096] {
        let Some(fixture) = common::generate(page_size)
        else {
            eprintln!("skipping SQLite schema interop: no python3/python with sqlite3 on this host");
            return;
        };

        assert_eq!(fixture.page_size, page_size);
        assert_eq!(validate_database(&fixture.bytes).expect("fixture header"), page_size);
        assert_eq!(fixture.manifest.page_size, page_size);
        assert_eq!(fixture.manifest.encoding, common::header_encoding(&fixture.bytes));

        let objects = read_schema(&fixture.bytes, TableLimits::default()).expect("read schema");
        assert_eq!(objects.len(), fixture.manifest.schema.len(), "schema row count for page size {page_size}");
        for (object, reference) in objects.iter().zip(&fixture.manifest.schema) {
            assert_eq!(kind_name(object.kind), reference.kind);
            assert_eq!(object.name, reference.name);
            assert_eq!(object.table_name, reference.table_name);
            assert_eq!(object.root_page.map(u32::from), Some(reference.root_page as u32));
            assert_eq!(object.sql.as_deref(), reference.sql.as_deref(), "CREATE source for {}", object.name);
        }

        for (table, expected) in common::EXPECTED_COLUMNS {
            let actual: Vec<_> = fixture.manifest.columns.iter().filter(|column| column.table == *table).collect();
            assert_eq!(actual.len(), expected.len(), "column count for {table} at page size {page_size}");
            for (column, expected) in actual.iter().zip(*expected) {
                assert_eq!(column.cid, expected.0);
                assert_eq!(column.name, expected.1);
                assert_eq!(column.declared_type, expected.2);
                assert_eq!(column.not_null, expected.3);
                assert_eq!(column.default.as_deref(), expected.4);
                assert_eq!(column.primary_key, expected.5);
            }
            let object =
                objects.iter().find(|object| object.name == *table).unwrap_or_else(|| panic!("schema object for {table}"));
            assert!(object.sql.is_some(), "reader must preserve CREATE source for {table}");
        }

        assert!(objects.iter().any(|object| object.kind == SchemaObjectKind::Table && object.name == "wo"));
        assert!(objects.iter().any(|object| object.kind == SchemaObjectKind::Index && object.name == "multi_amount_idx"));
        assert!(objects.iter().any(|object| object.kind == SchemaObjectKind::View && object.name == "v_simple"));
        assert!(
            objects.iter().any(|object| object.kind == SchemaObjectKind::Index && object.sql.is_none()),
            "automatic index SQL is NULL in sqlite_master"
        );
    }
}

#[test]
fn rowid_scans_match_upstream_sqlite_for_both_page_sizes() {
    for page_size in [1024, 4096] {
        let Some(fixture) = common::generate(page_size)
        else {
            eprintln!("skipping SQLite rowid interop: no python3/python with sqlite3 on this host");
            return;
        };
        let encoding = match common::header_encoding(&fixture.bytes) {
            "utf8" => TextEncoding::Utf8,
            "utf16le" => TextEncoding::Utf16Le,
            "utf16be" => TextEncoding::Utf16Be,
            other => panic!("unexpected fixture encoding {other}"),
        };
        assert_eq!(fixture.manifest.tables.len(), 7);
        for reference in fixture.manifest.tables.iter().filter(|table| table.name != "affinity") {
            let rows = read_named_table(&fixture.bytes, &reference.name, TableLimits::default())
                .unwrap_or_else(|error| panic!("read {} at page size {page_size}: {error}", reference.name));
            assert_eq!(rows.len(), reference.rows.len(), "row count for {} at {page_size}", reference.name);
            for (row, expected) in rows.iter().zip(&reference.rows) {
                assert_eq!(row.rowid, expected.rowid, "rowid for {} at {page_size}", reference.name);
                let values = decode_record(&row.payload, encoding, RecordLimits::default())
                    .unwrap_or_else(|error| panic!("decode {} rowid {}: {error}", reference.name, row.rowid));
                assert_eq!(values.len(), expected.values.len(), "column count for {} rowid {}", reference.name, row.rowid);
                for (index, (value, expected)) in values.iter().zip(&expected.values).enumerate() {
                    assert!(
                        matches_reference(value, expected),
                        "{} rowid {} column {index}: reader {value:?} vs upstream {expected:?}",
                        reference.name,
                        row.rowid
                    );
                }
            }
        }
    }
}

#[test]
fn real_affinity_projection_is_not_simulated() {
    for page_size in [1024, 4096] {
        let Some(fixture) = common::generate(page_size)
        else {
            eprintln!("skipping SQLite affinity check: no python3/python with sqlite3 on this host");
            return;
        };
        // `INSERT INTO affinity(v) VALUES(0.0)` is stored with the integer serial type
        // because SQLite normalizes integral REAL inputs, while `SELECT v` projects it
        // back as REAL 0.0 through column affinity. The reader exposes disk storage,
        // so it reports Integer(0). This is a documented boundary, not a bug.
        let reference = fixture.manifest.tables.iter().find(|table| table.name == "affinity").expect("affinity fixture table");
        assert_eq!(reference.rows[0].values, [ReferenceValue::Null, ReferenceValue::Real(0)]);

        let rows = read_named_table(&fixture.bytes, "affinity", TableLimits::default()).expect("read affinity");
        let values = decode_record(&rows[0].payload, TextEncoding::Utf8, RecordLimits::default()).expect("decode affinity");
        assert_eq!(values, [RecordValue::Null, RecordValue::Integer(0)]);
    }
}

#[test]
fn without_rowid_view_and_missing_table_are_explicit_errors() {
    for page_size in [1024, 4096] {
        let Some(fixture) = common::generate(page_size)
        else {
            eprintln!("skipping SQLite limitation checks: no python3/python with sqlite3 on this host");
            return;
        };
        let index_error = read_named_table(&fixture.bytes, "wo", TableLimits::default()).unwrap_err().to_string();
        assert!(index_error.contains("index b-tree scan"), "WITHOUT ROWID must be rejected explicitly: {index_error}");

        let view_error = read_named_table(&fixture.bytes, "v_simple", TableLimits::default()).unwrap_err().to_string();
        assert!(view_error.contains("view execution"), "views must not be evaluated: {view_error}");

        let missing = read_named_table(&fixture.bytes, "missing_table", TableLimits::default()).unwrap_err().to_string();
        assert!(missing.contains("not found"), "missing tables must be reported: {missing}");

        let schema = read_schema(&fixture.bytes, TableLimits::default()).expect("read schema");
        let master = read_named_table(&fixture.bytes, "sqlite_schema", TableLimits::default()).expect("schema alias");
        assert_eq!(master.len(), schema.len());

        let limit_error = read_named_table(&fixture.bytes, "types", TableLimits { max_rows: 1, ..Default::default() })
            .unwrap_err()
            .to_string();
        assert!(limit_error.contains("row limit"), "scan limits must stay enforced: {limit_error}");
    }
}

fn matches_reference(value: &RecordValue<'_>, expected: &ReferenceValue) -> bool {
    match (value, expected) {
        (RecordValue::Null, ReferenceValue::Null) => true,
        (RecordValue::Integer(actual), ReferenceValue::Integer(expected)) => actual == expected,
        (RecordValue::Real(actual), ReferenceValue::Real(expected)) => actual.to_bits() == *expected,
        (RecordValue::Text(actual), ReferenceValue::Text(expected)) => actual == expected,
        (RecordValue::Blob(actual), ReferenceValue::Blob(expected)) => *actual == expected.as_slice(),
        _ => false,
    }
}
