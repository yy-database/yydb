//! Failure matrix at the `Database` boundary: partial write, corruption and
//! recovery idempotency. Disk-full and lost-sync injection live in
//! `yydb-formats/tests/wal_durability.rs` because that is where the WAL sink is
//! pluggable.

use std::path::{Path, PathBuf};

use yydb_formats::{WAL_RECORD_HEADER_BYTES, parse_wal, wal_frame_offsets};
use yydb_storage::{Database, JournalMode, Snapshot, wal_path};
use yydb_types::Error;

fn temp_db(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yydb-failure-{}-{}", std::process::id(), label));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("main.ydb")
}

fn cleanup(path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::remove_dir_all(parent);
    }
}

fn snapshot(records: &[(&str, &[u8])]) -> Snapshot {
    Snapshot { metadata: Default::default(), records: records.iter().map(|(k, v)| ((*k).to_string(), v.to_vec())).collect() }
}

/// Two committed transactions plus the byte offsets and commit frame indices.
fn two_commits(path: &Path) -> (Vec<u8>, Vec<usize>, Vec<usize>) {
    let mut db = Database::open(path, JournalMode::Wal).unwrap();
    db.commit(&snapshot(&[("alpha", b"one")]), &[]).unwrap();
    db.commit(&snapshot(&[("alpha", b"one"), ("beta", b"two")]), &[]).unwrap();
    drop(db);
    let bytes = std::fs::read(wal_path(path)).unwrap();
    let offsets = wal_frame_offsets(&bytes).unwrap();
    let wal = parse_wal(&bytes).unwrap();
    let commits: Vec<usize> =
        wal.frames.iter().enumerate().filter(|(_, frame)| frame.frame_type == 0x04).map(|(index, _)| index).collect();
    assert_eq!(commits.len(), 2);
    (bytes, offsets, commits)
}

#[test]
fn partial_write_keeps_the_last_committed_transaction() {
    let path = temp_db("partial");
    let (bytes, offsets, commits) = two_commits(&path);
    let cut = offsets[commits[1]] + WAL_RECORD_HEADER_BYTES + 12;
    std::fs::write(wal_path(&path), &bytes[..cut]).unwrap();

    let mut db = Database::open(&path, JournalMode::Wal).unwrap();
    assert_eq!(std::fs::metadata(wal_path(&path)).unwrap().len(), offsets[commits[1]] as u64);
    let recovered = db.snapshot().unwrap();
    assert_eq!(recovered.records.get("alpha").map(Vec::as_slice), Some(b"one".as_slice()));
    assert_eq!(recovered.records.get("beta"), None);
    cleanup(&path);
}

#[test]
fn corrupt_wal_rejects_open_without_truncation() {
    let path = temp_db("corrupt");
    let (mut bytes, offsets, commits) = two_commits(&path);
    bytes[offsets[commits[1]] + WAL_RECORD_HEADER_BYTES + 2] ^= 0xFF;
    std::fs::write(wal_path(&path), &bytes).unwrap();

    let error = Database::open(&path, JournalMode::Wal).err().unwrap();
    assert!(matches!(error, Error::Corrupt("wal record checksum mismatch")));
    assert_eq!(
        std::fs::metadata(wal_path(&path)).unwrap().len(),
        bytes.len() as u64,
        "corruption must stay on disk for inspection"
    );
    cleanup(&path);
}

#[test]
fn recovery_is_idempotent_across_passes() {
    let path = temp_db("idempotent");
    let (bytes, offsets, commits) = two_commits(&path);
    let cut = offsets[commits[1]] + 20;
    std::fs::write(wal_path(&path), &bytes[..cut]).unwrap();

    let mut db = Database::open(&path, JournalMode::Wal).unwrap();
    let before = db.snapshot().unwrap();
    assert_eq!(std::fs::metadata(wal_path(&path)).unwrap().len(), offsets[commits[1]] as u64);

    let first = db.recover().unwrap();
    let second = db.recover().unwrap();
    assert_eq!(first, second, "a repeated recovery reaches the same report");
    assert_eq!(first.dropped_torn_bytes, 0);
    assert_eq!(db.snapshot().unwrap(), before, "a repeated recovery leaves the data unchanged");
    cleanup(&path);
}
