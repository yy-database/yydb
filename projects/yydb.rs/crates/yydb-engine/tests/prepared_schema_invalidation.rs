//! Prepared statement schema-fingerprint lifecycle.
//!
//! prepare → capture fingerprint + revision → execute (same schema) → ok →
//! migrate schema → execute → SchemaInvalidated → re-prepare → ok again.

use std::collections::BTreeMap;

use yydb_engine::{Connection, catalog::RenameMap};

const INITIAL_SCHEMA: &str = "table User { @@id: i64, name: utf8, active: bool }";
const MIGRATED_SCHEMA: &str = "table User { @@id: i64, name: utf8, active: bool, status: bool }";

#[test]
fn prepare_vos_captures_schema_fingerprint_and_revision() {
    let conn = Connection::open_in_memory().unwrap();
    conn.ensure_schema(INITIAL_SCHEMA).unwrap();

    let stmt = conn.prepare_vos("User.all().collect()").unwrap();
    assert!(!stmt.schema_fingerprint.is_empty(), "prepare_vos must populate fingerprint");
    assert!(stmt.schema_revision > 0, "prepare_vos must populate revision");

    // Verify the fingerprint is stable FNV-1a over the schema document.
    let direct_fp = yydb_query::fingerprint(INITIAL_SCHEMA);
    assert_eq!(stmt.schema_fingerprint, direct_fp);

    // A second prepare on the same schema produces the same fingerprint.
    let stmt2 = conn.prepare_vos("User.all().collect()").unwrap();
    assert_eq!(stmt.schema_fingerprint, stmt2.schema_fingerprint);
}

#[test]
fn prepare_vos_leaves_fields_empty_when_no_schema_bound() {
    let conn = Connection::open_in_memory().unwrap();
    let stmt = conn.prepare_vos("User.all().collect()").unwrap();
    assert!(stmt.schema_fingerprint.is_empty(), "no schema → empty fingerprint");
    assert_eq!(stmt.schema_revision, 0, "no schema → zero revision");
}

#[test]
fn query_prepared_succeeds_when_schema_unchanged() {
    let conn = Connection::open_in_memory().unwrap();
    conn.ensure_schema(INITIAL_SCHEMA).unwrap();

    let stmt = conn.prepare_vos("User.all().collect()").unwrap();
    let rows = conn.query_prepared(&stmt, &BTreeMap::new()).unwrap();
    assert!(rows.is_empty());

    // Repeated executions keep working.
    conn.query_prepared(&stmt, &BTreeMap::new()).unwrap();
    conn.query_prepared(&stmt, &BTreeMap::new()).unwrap();
}

#[test]
fn query_prepared_rejects_after_schema_migration() {
    let conn = Connection::open_in_memory().unwrap();
    conn.ensure_schema(INITIAL_SCHEMA).unwrap();

    let stmt = conn.prepare_vos("User.all().collect()").unwrap();
    let fingerprint_before = stmt.schema_fingerprint.clone();
    let revision_before = stmt.schema_revision;

    // First execution succeeds.
    conn.query_prepared(&stmt, &BTreeMap::new()).unwrap();

    // Migrate schema — document changes, so fingerprint will change.
    conn.migrate_schema(MIGRATED_SCHEMA, &RenameMap::default()).unwrap();

    // Now the old statement must be rejected with SchemaInvalidated.
    let err = conn.query_prepared(&stmt, &BTreeMap::new()).unwrap_err();
    assert_eq!(err.code(), "yydb.schema.invalidated");
    assert_eq!(err.operation(), "query");
    assert!(!err.retryable(), "schema invalidation is non-retryable");

    let params = err.params().expect("SchemaInvalidated carries params");
    let kv: BTreeMap<&str, String> = params.into_iter().collect();
    assert_eq!(kv.get("expected_fingerprint").unwrap(), &fingerprint_before);
    assert_ne!(kv.get("current_fingerprint").unwrap(), &fingerprint_before);
    assert_eq!(kv.get("schema_revision").unwrap(), &revision_before.to_string());

    // A freshly prepared statement after migration works fine.
    let fresh = conn.prepare_vos("User.all().collect()").unwrap();
    assert_ne!(fresh.schema_fingerprint, fingerprint_before);
    conn.query_prepared(&fresh, &BTreeMap::new()).unwrap();
}

#[test]
fn query_prepared_passes_through_when_statement_has_no_schema_context() {
    // A statement prepared via the pure yydb_query::prepare (no connection
    // context) has empty fingerprint and must never trigger the invalidation
    // gate — the gate only fires when the caller intentionally bound the
    // statement to a live schema.
    let conn = Connection::open_in_memory().unwrap();
    conn.ensure_schema(INITIAL_SCHEMA).unwrap();

    let stmt = yydb_query::prepare("User.all().collect()").unwrap();
    assert!(stmt.schema_fingerprint.is_empty());
    assert_eq!(stmt.schema_revision, 0);

    // Should execute just fine against the current schema.
    let rows = conn.query_prepared(&stmt, &BTreeMap::new()).unwrap();
    assert!(rows.is_empty());

    // And even after migration, the no-context statement keeps executing
    // because it never claimed to be bound to any particular schema.
    conn.migrate_schema(MIGRATED_SCHEMA, &RenameMap::default()).unwrap();
    conn.query_prepared(&stmt, &BTreeMap::new()).unwrap();
}
