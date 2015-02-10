//! Single-writer, multi-reader locking and fence epochs.

use std::path::{Path, PathBuf};

use yydb_storage::{Database, JournalMode, LockMode, Snapshot, WriterLock};
use yydb_types::Error;

fn temp_db(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yydb-lock-{}-{}", std::process::id(), label));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("main.ydb")
}

fn lock_path(db: &Path) -> PathBuf {
    let mut path = db.as_os_str().to_owned();
    path.push("-lock");
    PathBuf::from(path)
}

fn cleanup(path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::remove_dir_all(parent);
    }
}

fn is_locked(error: Error) -> bool {
    matches!(error, Error::Io(ref io_error) if io_error.kind() == std::io::ErrorKind::WouldBlock)
}

#[test]
fn writer_lock_is_exclusive_and_fenced() {
    let path = temp_db("exclusive");
    let first = WriterLock::acquire(&path).unwrap();
    assert_eq!(first.mode(), LockMode::Writer);
    assert_eq!(first.fence(), 1);

    assert!(is_locked(WriterLock::acquire(&path).err().unwrap()));
    drop(first);

    let second = WriterLock::acquire(&path).unwrap();
    assert_eq!(second.fence(), 2, "a restart advances the fence epoch");
    cleanup(&path);
}

#[test]
fn readers_share_and_exclude_the_writer() {
    let path = temp_db("readers");
    let first = WriterLock::acquire_reader(&path).unwrap();
    let second = WriterLock::acquire_reader(&path).unwrap();
    assert_eq!(first.mode(), LockMode::Reader);
    assert_eq!(first.fence(), 0, "a reader never advances the fence");
    assert_eq!(second.fence(), 0);

    assert!(is_locked(WriterLock::acquire(&path).err().unwrap()));
    drop(first);
    drop(second);

    let writer = WriterLock::acquire(&path).unwrap();
    assert_eq!(writer.fence(), 1);
    cleanup(&path);
}

#[test]
fn interrupted_lock_metadata_allows_restart() {
    let path = temp_db("interrupted");
    std::fs::write(lock_path(&path), b"YYLK\x00").unwrap();

    let recovered = WriterLock::acquire(&path).unwrap();
    assert_eq!(recovered.fence(), 1, "an incomplete acquisition restarts the epoch");
    drop(recovered);

    let next = WriterLock::acquire(&path).unwrap();
    assert_eq!(next.fence(), 2);
    cleanup(&path);
}

#[test]
fn foreign_lock_metadata_is_rejected() {
    let path = temp_db("foreign");
    std::fs::write(lock_path(&path), b"NOTLOCK!").unwrap();
    let error = WriterLock::acquire(&path).err().unwrap();
    assert!(matches!(error, Error::Corrupt("writer lock metadata is not YYLK")));
    cleanup(&path);
}

#[test]
fn writer_crash_in_critical_section_can_continue() {
    let path = temp_db("crash");
    let first_fence;
    {
        let mut db = Database::open(&path, JournalMode::Wal).unwrap();
        first_fence = db.writer_fence();
        let mut snapshot = Snapshot::default();
        snapshot.records.insert("alpha".to_string(), b"one".to_vec());
        db.commit(&snapshot, &[]).unwrap();
        // Dropping without a checkpoint models a crash that releases the OS lock.
    }

    let mut restarted = Database::open(&path, JournalMode::Wal).unwrap();
    assert!(restarted.writer_fence() > first_fence, "the restart must observe a newer fence");
    let recovered = restarted.snapshot().unwrap();
    assert_eq!(recovered.records.get("alpha").map(Vec::as_slice), Some(b"one".as_slice()));

    let mut next = recovered;
    next.records.insert("beta".to_string(), b"two".to_vec());
    restarted.commit(&next, &[]).unwrap();
    drop(restarted);

    let mut reopened = Database::open(&path, JournalMode::Wal).unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert_eq!(snapshot.records.get("alpha").map(Vec::as_slice), Some(b"one".as_slice()));
    assert_eq!(snapshot.records.get("beta").map(Vec::as_slice), Some(b"two".as_slice()));
    assert_eq!(reopened.recover().unwrap().dropped_torn_bytes, 0);
    cleanup(&path);
}
