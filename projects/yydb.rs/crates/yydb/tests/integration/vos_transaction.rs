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
fn nested_begin_is_rejected() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");
    let err = conn.begin().expect_err("nested begin");
    assert!(err.to_string().contains("transaction already open"));
    conn.rollback().expect("rollback");
}

#[test]
fn commit_without_open_transaction_errors() {
    let conn = Connection::open_in_memory().expect("open");
    let err = conn.commit().expect_err("commit without txn");
    assert!(err.to_string().contains("no open transaction"));
}

#[test]
fn rollback_discards_kv_writes() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");
    conn.put("key", b"value").expect("put");
    assert_eq!(conn.get("key").expect("read snapshot"), Some(b"value".to_vec()));
    conn.rollback().expect("rollback");
    assert_eq!(conn.get("key").expect("read after rollback"), None);
}

#[test]
fn commit_persists_kv_writes() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");
    conn.put("key", b"value").expect("put");
    conn.commit().expect("commit");
    assert_eq!(conn.get("key").expect("read after commit"), Some(b"value".to_vec()));
}

// ── Savepoint stack ────────────────────────────────────────────────────

#[test]
fn savepoint_rollback_restores_buffer_and_drops_nested_savepoints() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");
    // Baseline write — we want to verify rollback-to does NOT undo this.
    conn.put("baseline", b"1").expect("put baseline");

    conn.savepoint("x").expect("savepoint x");
    conn.put("after_x", b"2").expect("put after x");

    conn.savepoint("y").expect("savepoint y");
    conn.put("after_y", b"3").expect("put after y");

    // ROLLBACK TO x must erase after_x and after_y, while baseline survives.
    conn.rollback_to_savepoint("x").expect("rollback to x");
    assert_eq!(conn.get("baseline").expect("baseline still present"), Some(b"1".to_vec()));
    assert_eq!(conn.get("after_x").expect("after x must be gone"), None);
    assert_eq!(conn.get("after_y").expect("after y must be gone"), None);

    // Nested savepoint `y` must also have been removed.
    let err = conn.rollback_to_savepoint("y").expect_err("y was truncated");
    assert!(err.to_string().contains("does not exist"));

    conn.commit().expect("commit");
    // Post commit: baseline persists
    assert_eq!(conn.get("baseline").expect("committed baseline"), Some(b"1".to_vec()));
    assert_eq!(conn.get("after_x").expect("after x never committed"), None);
}

#[test]
fn release_savepoint_removes_name_without_undoing_writes() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");
    conn.put("before", b"1").expect("put");

    conn.savepoint("sp").expect("savepoint");
    conn.put("after", b"2").expect("put after sp");
    conn.release_savepoint("sp").expect("release");

    // The write `after` was NOT undone by RELEASE.
    assert_eq!(conn.get("after").expect("after write kept"), Some(b"2".to_vec()));
    // Subsequent ROLLBACK TO the released name must fail.
    let err = conn.rollback_to_savepoint("sp").expect_err("released name rejected");
    assert!(err.to_string().contains("does not exist"));

    conn.commit().expect("commit");
    assert_eq!(conn.get("after").expect("committed"), Some(b"2".to_vec()));
}

#[test]
fn nested_savepoint_rollback_to_outer_only_removes_outer_and_above() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");
    conn.savepoint("outer").expect("outer");
    conn.savepoint("inner").expect("inner");
    conn.put("k", b"v").expect("put");

    conn.rollback_to_savepoint("outer").expect("rollback to outer");
    // `inner` must have been dropped as it sits above `outer`.
    let err = conn.rollback_to_savepoint("inner").expect_err("inner truncated");
    assert!(err.to_string().contains("does not exist"));
    // `outer` itself is also gone (consumed by rollback_to_savepoint).
    let err = conn.rollback_to_savepoint("outer").expect_err("outer consumed");
    assert!(err.to_string().contains("does not exist"));

    conn.commit().expect("commit");
}

// ── Savepoint error paths ─────────────────────────────────────────────

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

#[test]
fn duplicate_savepoint_name_is_rejected() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");
    conn.savepoint("dup").expect("first");
    let err = conn.savepoint("dup").expect_err("duplicate");
    assert!(err.to_string().contains("already exists"));
    conn.rollback().expect("cleanup");
}

#[test]
fn rollback_to_unknown_savepoint_is_rejected() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");
    let err = conn.rollback_to_savepoint("ghost").expect_err("unknown");
    assert!(err.to_string().contains("does not exist"));
    conn.rollback().expect("cleanup");
}

#[test]
fn release_unknown_savepoint_is_rejected() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");
    let err = conn.release_savepoint("ghost").expect_err("unknown");
    assert!(err.to_string().contains("does not exist"));
    conn.rollback().expect("cleanup");
}

#[test]
fn commit_clears_savepoint_stack() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");
    conn.savepoint("sp").expect("create");
    conn.commit().expect("commit");

    // After commit, savepoint stack is cleared — a new begin should not find it.
    conn.begin().expect("fresh begin");
    let err = conn.rollback_to_savepoint("sp").expect_err("stack was cleared by commit");
    assert!(err.to_string().contains("does not exist"));
    conn.rollback().expect("cleanup");
}

#[test]
fn rollback_clears_savepoint_stack() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");
    conn.savepoint("sp").expect("create");
    conn.rollback().expect("rollback");

    conn.begin().expect("fresh begin");
    let err = conn.rollback_to_savepoint("sp").expect_err("stack was cleared by rollback");
    assert!(err.to_string().contains("does not exist"));
    conn.rollback().expect("cleanup");
}
