//! Transaction/savepoint semantics through the `yydb::Connection` facade.
//!
//! Most of this surface migrated to the TypeScript binding suite
//! (`statement-txn-kv` and `txn-savepoint-matrix` in `@yydb/yydb`), which
//! covers begin/commit/rollback visibility, nested-transaction refusal, the
//! full savepoint chain (rollback-to, consumption, release, name reuse, full
//! rollback) and stack clearing on commit/rollback. The tests below stay
//! Rust-side because they assert behaviors the host API cannot reach:
//! `in_transaction` state introspection and raw commit/rollback/savepoint
//! calls without an open transaction (the host only exposes transactions
//! through `Database.transaction`).

use yydb::Connection;

#[test]
fn begin_rollback_toggles_transaction_state() {
    let conn = Connection::open_in_memory().expect("open");
    assert!(!conn.in_transaction());
    conn.begin().expect("begin");
    assert!(conn.in_transaction());
    conn.rollback().expect("rollback");
    assert!(!conn.in_transaction());
}

#[test]
fn commit_without_open_transaction_errors() {
    let conn = Connection::open_in_memory().expect("open");
    let err = conn.commit().expect_err("commit without txn");
    assert!(err.to_string().contains("no open transaction"));
}

#[test]
fn savepoint_requires_open_transaction() {
    let conn = Connection::open_in_memory().expect("open");
    let err = conn.savepoint("sp").expect_err("savepoint outside txn");
    assert!(err.to_string().contains("no open transaction"));

    let err = conn.rollback_to_savepoint("sp").expect_err("rollback-to outside txn");
    assert!(err.to_string().contains("no open transaction"));

    let err = conn.release_savepoint("sp").expect_err("release outside txn");
    assert!(err.to_string().contains("no open transaction"));
}
