//! Connection close lifecycle: a prepared statement handle becomes invalid
//! the moment its owning connection (or a parent connection it was derived
//! from via `Connection::session`) is closed.

use std::collections::BTreeMap;

use yydb_engine::Connection;

const USER_SCHEMA: &str = "table User { @@id: i64, name: utf8 }";

#[test]
fn query_prepared_rejects_after_connection_close() {
    let conn = Connection::open_in_memory().unwrap();
    conn.ensure_schema(USER_SCHEMA).unwrap();
    conn.execute_vos_write(r#"User.insert({ id: 1, name: "alice" })"#).unwrap();

    let stmt = conn.prepare_vos("User.all().collect()").unwrap();
    assert!(!conn.is_closed(), "connection must start alive");

    // Happy path works before close.
    let rows = conn.query_prepared(&stmt, &BTreeMap::new()).unwrap();
    assert_eq!(rows.len(), 1);

    conn.close();
    assert!(conn.is_closed(), "connection must flip closed flag");

    // Stale PreparedStatement handle now fails with SessionClosed.
    let err = conn.query_prepared(&stmt, &BTreeMap::new()).unwrap_err();
    assert_eq!(err.code(), "yydb.session.closed");
    assert_eq!(err.operation(), "session");
}

#[test]
fn prepare_vos_rejects_after_connection_close() {
    let conn = Connection::open_in_memory().unwrap();
    conn.close();

    let err = conn.prepare_vos("User.all().collect()").unwrap_err();
    assert_eq!(err.code(), "yydb.session.closed");
}

#[test]
fn execute_vos_write_rejects_after_connection_close() {
    let conn = Connection::open_in_memory().unwrap();
    conn.close();

    let err = conn.execute_vos_write("User.all().collect()").unwrap_err();
    assert_eq!(err.code(), "yydb.session.closed");
}

#[test]
fn session_derived_from_closed_parent_is_also_closed() {
    let parent = Connection::open_in_memory().unwrap();
    parent.ensure_schema(USER_SCHEMA).unwrap();
    parent.execute_vos_write(r#"User.insert({ id: 42, name: "bob" })"#).unwrap();

    let child = parent.session();
    let stmt = child.prepare_vos("User.all().collect()").unwrap();
    let rows = child.query_prepared(&stmt, &BTreeMap::new()).unwrap();
    assert_eq!(rows.len(), 1);

    // Closing the parent invalidates the shared Arc<AtomicBool>, so the
    // derived session sees it instantly.
    parent.close();
    assert!(parent.is_closed());
    assert!(child.is_closed(), "derived session must observe parent close");

    let err = child.query_prepared(&stmt, &BTreeMap::new()).unwrap_err();
    assert_eq!(err.code(), "yydb.session.closed");
}

#[test]
fn close_is_idempotent() {
    let conn = Connection::open_in_memory().unwrap();
    conn.close();
    conn.close(); // second call must not panic or corrupt state
    assert!(conn.is_closed());
}
