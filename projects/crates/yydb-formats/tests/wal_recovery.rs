//! Startup recovery over partial writes, torn tails and damaged records.

use std::path::{Path, PathBuf};

use yydb_formats::{
    FilePager, WAL_RECORD_HEADER_BYTES, WalRecordError, parse_wal, scan_wal, wal_frame_offsets, wal_sidecar_path,
};
use yydb_types::Error;

fn temp_db(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yydb-recovery-{}-{}", std::process::id(), label));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("main.ydb")
}

fn cleanup(path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::remove_dir_all(parent);
    }
}

/// Two committed transactions: `alpha` then `beta`.
fn committed_wal(label: &str) -> (PathBuf, Vec<u8>, Vec<usize>, Vec<usize>) {
    let path = temp_db(label);
    let mut pager = FilePager::open(&path, true).unwrap();
    pager.put_kv(b"alpha", b"one").unwrap();
    pager.put_kv(b"beta", b"two").unwrap();
    drop(pager);
    let bytes = std::fs::read(wal_sidecar_path(&path)).unwrap();
    let offsets = wal_frame_offsets(&bytes).unwrap();
    let wal = parse_wal(&bytes).unwrap();
    let commits: Vec<usize> =
        wal.frames.iter().enumerate().filter(|(_, frame)| frame.frame_type == 0x04).map(|(index, _)| index).collect();
    assert_eq!(commits.len(), 2);
    (path, bytes, offsets, commits)
}

fn write_wal(path: &Path, bytes: &[u8]) {
    std::fs::write(wal_sidecar_path(path), bytes).unwrap();
}

#[test]
fn recovery_truncates_partial_page_write() {
    let (path, bytes, offsets, commits) = committed_wal("partial-page");
    let cut = offsets[commits[0] + 1] + WAL_RECORD_HEADER_BYTES + 10;
    write_wal(&path, &bytes[..cut]);

    let mut pager = FilePager::open(&path, true).unwrap();
    let report = pager.last_recovery();
    assert_eq!(report.dropped_torn_bytes, (cut - offsets[commits[0] + 1]) as u64);
    assert_eq!(report.frames, commits[0] + 1, "only the first transaction survives");
    assert_eq!(report.committed_transactions, 1);
    assert_eq!(std::fs::metadata(wal_sidecar_path(&path)).unwrap().len(), offsets[commits[0] + 1] as u64);
    assert_eq!(pager.get_kv(b"alpha").unwrap().as_deref(), Some(b"one".as_slice()));
    assert_eq!(pager.get_kv(b"beta").unwrap(), None, "an incomplete transaction must not be applied");

    let second = pager.recover().unwrap();
    assert_eq!(second.dropped_torn_bytes, 0, "a second recovery must not truncate more bytes");
    assert_eq!(second.frames, report.frames);
    assert_eq!(second.committed_lsn, report.committed_lsn);
    assert_eq!(pager.get_kv(b"alpha").unwrap().as_deref(), Some(b"one".as_slice()));

    pager.put_kv(b"gamma", b"three").unwrap();
    drop(pager);

    let mut reopened = FilePager::open(&path, true).unwrap();
    assert_eq!(reopened.get_kv(b"alpha").unwrap().as_deref(), Some(b"one".as_slice()));
    assert_eq!(reopened.get_kv(b"beta").unwrap(), None);
    assert_eq!(reopened.get_kv(b"gamma").unwrap().as_deref(), Some(b"three".as_slice()));
    cleanup(&path);
}

#[test]
fn recovery_truncates_trailing_half_commit_record() {
    let (path, bytes, offsets, commits) = committed_wal("half-commit");
    let commit_start = offsets[commits[1]];
    let declared = u32::from_le_bytes(bytes[commit_start + 9..commit_start + 13].try_into().unwrap()) as usize;
    let cut = commit_start + WAL_RECORD_HEADER_BYTES + declared / 2;
    write_wal(&path, &bytes[..cut]);

    let mut pager = FilePager::open(&path, true).unwrap();
    assert!(pager.last_recovery().dropped_torn_bytes > 0);
    assert_eq!(pager.last_recovery().committed_transactions, 1);
    assert_eq!(pager.get_kv(b"alpha").unwrap().as_deref(), Some(b"one".as_slice()));
    assert_eq!(pager.get_kv(b"beta").unwrap(), None);
    cleanup(&path);
}

#[test]
fn recovery_rejects_bad_crc_on_last_record() {
    let (path, mut bytes, offsets, commits) = committed_wal("bad-crc");
    bytes[offsets[commits[1]] + WAL_RECORD_HEADER_BYTES + 4] ^= 0xFF;
    write_wal(&path, &bytes);

    let error = FilePager::open(&path, true).err().unwrap();
    assert!(matches!(error, Error::Corrupt("wal record checksum mismatch")));
    assert_eq!(
        scan_wal(&bytes).unwrap_err(),
        WalRecordError::RecordChecksumMismatch { offset: offsets[commits[1]] },
        "the damaged record reports its own offset"
    );
    assert_eq!(
        std::fs::metadata(wal_sidecar_path(&path)).unwrap().len(),
        bytes.len() as u64,
        "corruption must not be truncated away as a torn tail"
    );
    cleanup(&path);
}

#[test]
fn recovery_rejects_multiple_corrupt_records() {
    let (path, mut bytes, offsets, commits) = committed_wal("multi-corrupt");
    bytes[offsets[commits[0]] + WAL_RECORD_HEADER_BYTES + 1] ^= 0xFF;
    bytes[offsets[commits[1]] + WAL_RECORD_HEADER_BYTES + 1] ^= 0xFF;
    write_wal(&path, &bytes);

    let error = FilePager::open(&path, true).err().unwrap();
    assert!(matches!(error, Error::Corrupt("wal record checksum mismatch")));
    assert_eq!(scan_wal(&bytes).unwrap_err(), WalRecordError::RecordChecksumMismatch { offset: offsets[commits[0]] });
    cleanup(&path);
}
