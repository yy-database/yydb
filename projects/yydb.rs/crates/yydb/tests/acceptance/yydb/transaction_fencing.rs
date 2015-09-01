// fixture: yydb.transaction_fencing

use std::sync::{Arc, Barrier};

use crate::fixtures::yydb::{cleanup, open_temp_db, temp_db};
use yydb::{Connection, Error, OpenFlags};

fn conflict_code(error: &Error) -> &str {
    assert!(matches!(error, Error::TransactionConflict { .. }), "expected a transaction conflict, got {error:?}");
    error.code()
}

#[test]
fn yydb_txn_commit_conflict_loses_no_committed_write() {
    let path = temp_db("txn-conflict-keeps-winner");
    let root = Connection::open_with_flags(&path, OpenFlags::wal()).expect("open");
    let first = root.session();
    let second = root.session();

    first.begin().expect("first begin");
    second.begin().expect("second begin");
    first.put("contended", b"one").expect("first put");
    first.commit().expect("first commit wins");
    second.put("contended", b"two").expect("second put stays buffered");

    let error = second.commit().err().expect("second commit must not silently succeed");
    assert_eq!(conflict_code(&error), "yydb.transaction.conflict");

    // Nothing from the losing transaction was published and the loser is out
    // of transaction scope: the connection stays usable through a fresh retry.
    assert_eq!(root.get("contended").expect("get"), Some(b"one".to_vec()));
    assert!(!second.in_transaction());
    assert!(second.rollback().is_err(), "a conflicted commit already ended the transaction");
    second.begin().expect("retry begin");
    assert_eq!(second.get("contended").expect("retry reads the winner"), Some(b"one".to_vec()));
    second.put("contended", b"two").expect("retry put");
    second.commit().expect("retry commit");
    assert_eq!(root.get("contended").expect("get"), Some(b"two".to_vec()));

    drop(first);
    drop(second);
    drop(root);
    cleanup(&path);
}

#[test]
fn yydb_txn_disjoint_writes_conflict_by_contract() {
    let path = temp_db("txn-disjoint-conflict");
    let root = Connection::open_with_flags(&path, OpenFlags::wal()).expect("open");
    let first = root.session();
    let second = root.session();

    first.begin().expect("first begin");
    second.begin().expect("second begin");
    first.put("only-first", b"a").expect("first put");
    second.put("only-second", b"b").expect("second put");
    first.commit().expect("first commit");
    let error = second.commit().err().expect("whole-snapshot fencing does not exempt disjoint keys");
    assert!(matches!(error, Error::TransactionConflict { .. }));

    assert_eq!(root.get("only-first").expect("get"), Some(b"a".to_vec()));
    assert_eq!(root.get("only-second").expect("get"), None, "the conflicted transaction published nothing");

    drop(first);
    drop(second);
    drop(root);
    cleanup(&path);
}

#[test]
fn yydb_txn_snapshot_is_stable_and_reads_outside_stay_fresh() {
    let path = temp_db("txn-stable-snapshot");
    let root = Connection::open_with_flags(&path, OpenFlags::wal()).expect("open");
    root.put("watched", b"v0").expect("seed");

    let reader = root.session();
    reader.begin().expect("begin");
    assert_eq!(reader.get("watched").expect("snapshot read"), Some(b"v0".to_vec()));

    root.put("watched", b"v1").expect("concurrent non-transactional write");

    // The open transaction keeps its begin-time snapshot while a plain read
    // outside any transaction always sees the latest committed state.
    assert_eq!(reader.get("watched").expect("in-transaction read"), Some(b"v0".to_vec()));
    assert_eq!(root.get("watched").expect("fresh read"), Some(b"v1".to_vec()));

    // A write published on the stale base must conflict, and the buffered
    // write must not leak into the durable state.
    reader.put("scratch", b"from-stale-base").expect("buffered write");
    let error = reader.commit().err().expect("commit on a stale base must conflict");
    assert!(matches!(error, Error::TransactionConflict { .. }));
    assert_eq!(root.get("scratch").expect("get"), None, "failed commit left no half-published state");
    assert_eq!(root.get("watched").expect("get"), Some(b"v1".to_vec()));

    drop(reader);
    drop(root);
    cleanup(&path);
}

#[test]
fn yydb_same_connection_read_write_txn_interleave() {
    let (conn, path) = open_temp_db("txn-interleave");

    // Read-your-writes inside the transaction, discard on rollback.
    conn.begin().expect("begin");
    conn.put("transient", b"buffered").expect("put");
    assert_eq!(conn.get("transient").expect("read-your-writes"), Some(b"buffered".to_vec()));
    conn.rollback().expect("rollback");
    assert_eq!(conn.get("transient").expect("get after rollback"), None);

    // A committed write becomes visible to other sessions only at commit.
    let peer = conn.session();
    conn.begin().expect("begin write txn");
    conn.put("durable", b"committed").expect("put");
    assert_eq!(peer.get("durable").expect("peer must not see the buffer"), None);
    conn.commit().expect("commit");
    assert_eq!(peer.get("durable").expect("peer read after commit"), Some(b"committed".to_vec()));

    // A read transaction interleaved with a committed write on the same
    // connection keeps its own snapshot, then starts fresh after it ends.
    conn.begin().expect("begin read txn");
    let snapshotted = conn.get("durable").expect("snapshot read");
    conn.rollback().expect("end read txn");
    conn.put("durable", b"updated").expect("write after read txn");
    assert_eq!(snapshotted, Some(b"committed".to_vec()));
    assert_eq!(conn.get("durable").expect("get"), Some(b"updated".to_vec()));

    drop(peer);
    drop(conn);
    cleanup(&path);
}

#[test]
fn yydb_schema_publish_conflicts_open_transaction() {
    let path = temp_db("txn-schema-conflict");
    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).expect("open");

    conn.begin().expect("begin");
    conn.put("row", b"buffered").expect("put");
    // Schema/DDL stays outside the transaction boundary and publishes at once,
    // which moves the durable commit base under the open transaction.
    conn.ensure_schema("table Setting { @@id: uuid, key: utf8 }").expect("schema publishes immediately");
    let error = conn.commit().err().expect("commit after a schema publish must conflict");
    assert!(matches!(error, Error::TransactionConflict { .. }));

    assert_eq!(conn.get("row").expect("get"), None, "the conflicted transaction published nothing");
    assert!(conn.schema().expect("schema").is_some(), "the schema publish itself stands");

    drop(conn);
    cleanup(&path);
}

#[test]
fn yydb_memory_backend_applies_the_same_fencing() {
    let root = Connection::open_in_memory().expect("open in memory");
    let first = root.session();
    let second = root.session();

    first.begin().expect("first begin");
    second.begin().expect("second begin");
    first.put("k", b"winner").expect("first put");
    first.commit().expect("first commit");
    second.put("k", b"loser").expect("second put");
    let error = second.commit().err().expect("in-memory commit must fence the same way");
    assert!(matches!(error, Error::TransactionConflict { .. }));
    assert_eq!(root.get("k").expect("get"), Some(b"winner".to_vec()));
}

#[test]
fn yydb_threaded_sessions_serialize_without_torn_state() {
    let path = temp_db("txn-threaded-sessions");
    let root = Arc::new(Connection::open_with_flags(&path, OpenFlags::wal()).expect("open"));
    let workers = 4;
    let writes_per_worker = 25;
    let barrier = Arc::new(Barrier::new(workers));

    let handles: Vec<_> = (0..workers)
        .map(|worker| {
            let root = Arc::clone(&root);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let session = root.session();
                barrier.wait();
                for index in 0..writes_per_worker {
                    let key = format!("worker-{worker}/{index}");
                    session.put(&key, format!("value-{index}").as_bytes()).expect("put");
                    assert_eq!(session.get(&key).expect("read back"), Some(format!("value-{index}").into_bytes()));
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("worker thread");
    }

    for worker in 0..workers {
        assert_eq!(
            root.get(&format!("worker-{worker}/24")).expect("final read"),
            Some(b"value-24".to_vec()),
            "worker {worker} lost its final committed write"
        );
    }

    drop(root);
    cleanup(&path);
}
