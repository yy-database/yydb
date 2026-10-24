//! Cross-checks the read-only reader against files produced by upstream SQLite.
//!
//! Fixtures are generated at test time through Python's bundled `sqlite3`
//! module. The reader result and the upstream `sqlite_master` /
//! `PRAGMA table_info` result are compared row by row and column by column.

mod common;

use sqlite::{SchemaObjectKind, TableLimits, read_schema, validate_database};

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
