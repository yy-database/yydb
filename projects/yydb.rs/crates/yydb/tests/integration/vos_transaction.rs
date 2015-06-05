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

// ── Savepoint cross-matrix: stepwise rollback-to chain ─────────────────

/// Rollback-to chain: x → write A → y → write B → ROLLBACK TO y (A kept, B
/// gone) → ROLLBACK TO x (A also gone). Verifies each named savepoint is a
/// true snapshot barrier and rollback-to is idempotent-clearing.
#[test]
fn nested_rollback_to_chain_restores_stepwise() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");

    conn.savepoint("x").expect("x");
    conn.put("A", b"a").expect("write A");

    conn.savepoint("y").expect("y");
    conn.put("B", b"b").expect("write B");

    // First rollback-to: y — B must vanish, A survives.
    conn.rollback_to_savepoint("y").expect("rollback to y");
    assert_eq!(conn.get("A").expect("A still alive"), Some(b"a".to_vec()));
    assert_eq!(conn.get("B").expect("B undone"), None);

    // Second rollback-to: x — A must also vanish now.
    conn.rollback_to_savepoint("x").expect("rollback to x");
    assert_eq!(conn.get("A").expect("A also undone"), None);
    assert_eq!(conn.get("B").expect("B gone all along"), None);

    conn.commit().expect("commit empty buffer");
}

// ── Savepoint cross-matrix: release + full rollback ────────────────────

/// RELEASE SAVEPOINT only removes the savepoint name; a subsequent full
/// transaction ROLLBACK still undoes every write — including writes that
/// happened *after* the RELEASE.
#[test]
fn release_savepoint_then_full_rollback_undoes_everything() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");

    conn.savepoint("sp").expect("sp");
    conn.put("w1", b"one").expect("write before release");
    conn.release_savepoint("sp").expect("release");
    conn.put("w2", b"two").expect("write after release");

    conn.rollback().expect("full rollback");
    assert_eq!(conn.get("w1").expect("w1 gone"), None);
    assert_eq!(conn.get("w2").expect("w2 gone"), None);
}

// ── Savepoint cross-matrix: triple nested rollback-to middle ───────────

/// Three-layer nesting: rollback-to the middle one must keep its prefix
/// (outer), discard everything after it, and drop every nested savepoint
/// strictly above the target — *including the target name itself*.
#[test]
fn triple_nested_savepoint_rollback_to_middle_restores_only_prefix_writes() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");

    conn.savepoint("a").expect("a");
    conn.put("w_a", b"1").expect("w_a");

    conn.savepoint("b").expect("b");
    conn.put("w_b", b"2").expect("w_b");

    conn.savepoint("c").expect("c");
    conn.put("w_c", b"3").expect("w_c");

    conn.rollback_to_savepoint("b").expect("rollback to b");

    // Prefix write survives; writes strictly after b are undone.
    assert_eq!(conn.get("w_a").expect("w_a kept"), Some(b"1".to_vec()));
    assert_eq!(conn.get("w_b").expect("w_b undone"), None);
    assert_eq!(conn.get("w_c").expect("w_c undone"), None);

    // Every savepoint at or above `b` must be gone (b itself is consumed,
    // c sits strictly above).
    let err = conn.rollback_to_savepoint("b").expect_err("b consumed");
    assert!(err.to_string().contains("does not exist"));
    let err = conn.rollback_to_savepoint("c").expect_err("c truncated");
    assert!(err.to_string().contains("does not exist"));
    // `a` is still reachable — it sits below b and was untouched.
    conn.rollback_to_savepoint("a").expect("a still reachable");

    conn.commit().expect("commit");
}

// ── Savepoint cross-matrix: release frees the name pool ────────────────

/// After RELEASE, the savepoint name returns to the pool — a fresh
/// SAVEPOINT with the same name must succeed.
#[test]
fn release_then_same_savepoint_name_is_allowed() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");

    conn.savepoint("x").expect("first x");
    conn.release_savepoint("x").expect("release");
    // Name must be free again after release.
    conn.savepoint("x").expect("re-create x after release");

    conn.commit().expect("commit");
}

// ── Savepoint cross-matrix: rollback-to consumes the named savepoint ───

/// A savepoint consumed by rollback-to can't be targeted again — even
/// though rollback-to restores the buffer, the name itself is gone.
#[test]
fn rollback_to_consumes_the_named_savepoint_name() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");

    conn.savepoint("x").expect("x");
    conn.put("k", b"v").expect("write");
    conn.rollback_to_savepoint("x").expect("first rollback-to x");
    assert_eq!(conn.get("k").expect("k undone"), None);

    // `x` itself is gone — no second rollback-to the same name.
    let err = conn.rollback_to_savepoint("x").expect_err("x already consumed");
    assert!(err.to_string().contains("does not exist"));

    conn.commit().expect("commit");
}

// ── Savepoint cross-matrix: empty savepoint no-op rollback-to ──────────

/// A savepoint captured before any writes: rollback-to it is a no-op on
/// the buffer — the transaction is still open and every subsequent write
/// after the empty savepoint just proceeds normally.
#[test]
fn empty_savepoint_then_rollback_to_is_buffer_noop() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");

    // Baseline write outside any savepoint.
    conn.put("baseline", b"1").expect("baseline");

    // Empty savepoint — nothing written since it was created.
    conn.savepoint("empty").expect("empty");
    conn.rollback_to_savepoint("empty").expect("rollback to empty");

    // Baseline must still be there; the no-op rollback-to didn't erase it.
    assert_eq!(conn.get("baseline").expect("baseline survives"), Some(b"1".to_vec()));
    assert!(conn.in_transaction(), "txn still open");

    conn.commit().expect("commit");
}

// ── Savepoint cross-matrix: release below a surviving inner savepoint ──

/// RELEASE of an outer savepoint does *not* disturb inner ones. Rollback-to
/// the inner savepoint must still succeed with the correct snapshot.
#[test]
fn release_outer_does_not_disturb_inner_savepoint() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");

    conn.savepoint("outer").expect("outer");
    conn.put("w", b"1").expect("write between");

    conn.savepoint("inner").expect("inner");
    conn.put("v", b"2").expect("write after inner");

    conn.release_savepoint("outer").expect("release outer");

    // `inner` must still be reachable and its snapshot must undo only
    // writes after it (i.e. `v`), keeping `w`.
    conn.rollback_to_savepoint("inner").expect("inner survived release of outer");
    assert_eq!(conn.get("w").expect("w survived"), Some(b"1".to_vec()));
    assert_eq!(conn.get("v").expect("v undone"), None);

    conn.commit().expect("commit");
}

// ── Savepoint cross-matrix: rollback_to above a release savepoint ──────

/// Rollback-to an outer savepoint that sits *above* a released inner one —
/// the released name is just a name; the snapshot at that outer barrier
/// still rules everything below it.
#[test]
fn rollback_to_outer_undoes_post_release_writes_in_other_branch() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");

    conn.put("before", b"1").expect("before outer snapshot");

    conn.savepoint("outer").expect("outer");
    conn.savepoint("inner").expect("inner");
    conn.put("mid", b"2").expect("between inner and release");
    conn.release_savepoint("inner").expect("release inner");
    conn.put("after", b"3").expect("after inner release");

    conn.rollback_to_savepoint("outer").expect("rollback to outer");

    // outer snapshot captured txn state *before* it was created — i.e. with
    // `before` already written. Everything after outer (mid, after) must
    // vanish, regardless of the inner release in between.
    assert_eq!(conn.get("before").expect("before outer kept"), Some(b"1".to_vec()));
    assert_eq!(conn.get("mid").expect("mid undone"), None);
    assert_eq!(conn.get("after").expect("after undone"), None);

    // inner name gone too — sits strictly above outer, truncated by rollback-to.
    let err = conn.rollback_to_savepoint("inner").expect_err("inner truncated");
    assert!(err.to_string().contains("does not exist"));

    conn.commit().expect("commit");
}

// ── Savepoint cross-matrix: commit before any savepoint still opens txn ─

/// Regression: ensure commit after begin (no savepoint) still flushes cleanly.
#[test]
fn commit_without_any_savepoint_after_begin_flushes_cleanly() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");
    conn.put("k", b"v").expect("write");
    assert!(conn.in_transaction());
    conn.commit().expect("commit");
    assert!(!conn.in_transaction());
    assert_eq!(conn.get("k").expect("committed"), Some(b"v".to_vec()));

    // Open a fresh txn, commit without any savepoint — must not carry
    // leftovers from the previous transaction state.
    conn.begin().expect("begin again");
    conn.put("k2", b"v2").expect("write k2");
    conn.commit().expect("second commit");
    assert_eq!(conn.get("k2").expect("k2 committed"), Some(b"v2".to_vec()));
}

// ── Savepoint cross-matrix: double savepoint same txn not allowed ──────

/// Duplicate savepoint name inside the same txn is rejected *even after*
/// a sibling savepoint was created/released in between — inner name pool
/// is only freed by explicit RELEASE or rollback-to.
#[test]
fn duplicate_savepoint_name_rejected_even_after_other_savepoint_created() {
    let conn = Connection::open_in_memory().expect("open");
    conn.begin().expect("begin");
    conn.savepoint("a").expect("a");
    conn.savepoint("b").expect("b");

    // Try to re-create `a` — still on the stack.
    let err = conn.savepoint("a").expect_err("duplicate a");
    assert!(err.to_string().contains("already exists"));

    conn.rollback_to_savepoint("b").expect("rollback to b");
    // `b` gone, `a` still alive. Duplicate `a` still rejected.
    let err = conn.savepoint("a").expect_err("duplicate a still");
    assert!(err.to_string().contains("already exists"));

    conn.rollback().expect("cleanup");
}
