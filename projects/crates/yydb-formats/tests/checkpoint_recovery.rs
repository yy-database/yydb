//! Checkpoint ordering, crash windows and checkpoint-LSN replay filtering.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use yydb_formats::{
    FilePager, PAGE_SIZE, WalFile, WalHeader, encode_frame, encode_wal_header, parse_page0, parse_wal, replay_wal_pages,
    replay_wal_pages_after, truncate_wal_records, wal_sidecar_path,
};

const DB_ID: [u8; 16] = [0x77; 16];

fn temp_db(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yydb-checkpoint-{}-{}", std::process::id(), label));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("main.ydb")
}

fn cleanup(path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::remove_dir_all(parent);
    }
}

fn checkpoint_lsn(path: &Path) -> u64 {
    parse_page0(&std::fs::read(path).unwrap()).unwrap().slot.checkpoint_lsn
}

#[test]
fn checkpoint_folds_wal_into_main_file() {
    let path = temp_db("fold");
    let mut pager = FilePager::open(&path, true).unwrap();
    pager.put_kv(b"alpha", b"one").unwrap();
    pager.put_kv(b"beta", b"two").unwrap();
    let wal_path = wal_sidecar_path(&path);
    assert!(wal_path.exists());

    pager.checkpoint().unwrap();
    assert!(!wal_path.exists(), "a completed checkpoint removes the WAL sidecar");
    assert!(checkpoint_lsn(&path) >= 4, "the page 0 header records the folded commit LSN");
    drop(pager);

    let mut reopened = FilePager::open(&path, true).unwrap();
    assert_eq!(reopened.get_kv(b"alpha").unwrap().as_deref(), Some(b"one".as_slice()));
    assert_eq!(reopened.get_kv(b"beta").unwrap().as_deref(), Some(b"two".as_slice()));
    cleanup(&path);
}

#[test]
fn checkpoint_crash_before_header_commit_keeps_wal_replayable() {
    let path = temp_db("before-header");
    let mut pager = FilePager::open(&path, true).unwrap();
    pager.put_kv(b"alpha", b"one").unwrap();
    pager.simulate_checkpoint_crash_before_header_commit().unwrap();
    drop(pager);

    let wal_path = wal_sidecar_path(&path);
    assert!(wal_path.exists(), "a crash before the page 0 commit must keep the WAL");
    assert_eq!(checkpoint_lsn(&path), 0, "uncommitted pages must not advance the checkpoint");

    let mut reopened = FilePager::open(&path, true).unwrap();
    assert_eq!(reopened.get_kv(b"alpha").unwrap().as_deref(), Some(b"one".as_slice()));
    cleanup(&path);
}

#[test]
fn checkpoint_crash_after_header_commit_does_not_replay_folded_records() {
    let path = temp_db("after-header");
    let mut pager = FilePager::open(&path, true).unwrap();
    pager.put_kv(b"alpha", b"one").unwrap();
    let wal_path = wal_sidecar_path(&path);
    let records = parse_wal(&std::fs::read(&wal_path).unwrap()).unwrap().frames.len();
    assert!(records > 0);

    pager.simulate_checkpoint_crash_after_header_commit().unwrap();
    drop(pager);

    assert!(wal_path.exists(), "the crash window leaves the WAL bytes in place");
    let committed = checkpoint_lsn(&path);
    assert!(committed >= 4, "the checkpoint LSN is durable in page 0");

    let mut reopened = FilePager::open(&path, true).unwrap();
    assert_eq!(reopened.get_kv(b"alpha").unwrap().as_deref(), Some(b"one".as_slice()));
    reopened.checkpoint().unwrap();
    assert!(!wal_path.exists());
    cleanup(&path);
}

#[test]
fn checkpoint_rotation_keeps_new_wal_lsns_above_the_checkpoint() {
    let path = temp_db("rotation-before-header");
    let mut pager = FilePager::open(&path, true).unwrap();
    pager.put_kv(b"baseline", b"ok").unwrap();
    pager.checkpoint().unwrap();
    let folded = checkpoint_lsn(&path);
    assert!(folded >= 4, "the baseline checkpoint is durable in page 0");
    drop(pager);

    let mut rotated = FilePager::open(&path, true).unwrap();
    rotated.put_kv(b"pending", b"wal").unwrap();
    drop(rotated);

    assert!(wal_sidecar_path(&path).exists(), "the rotated WAL still holds the pending commit");
    let mut reopened = FilePager::open(&path, true).unwrap();
    assert_eq!(reopened.get_kv(b"baseline").unwrap().as_deref(), Some(b"ok".as_slice()));
    assert_eq!(
        reopened.get_kv(b"pending").unwrap().as_deref(),
        Some(b"wal".as_slice()),
        "a WAL created after a checkpoint must not reuse LSNs at or below the checkpoint"
    );
    cleanup(&path);
}

#[test]
fn checkpoint_crash_after_wal_truncation_keeps_folded_data() {
    let path = temp_db("after-truncate");
    let mut pager = FilePager::open(&path, true).unwrap();
    pager.put_kv(b"alpha", b"one").unwrap();
    pager.simulate_checkpoint_crash_after_header_commit().unwrap();
    drop(pager);

    let wal = wal_sidecar_path(&path);
    assert!(wal.exists(), "the crash window between truncation and unlink leaves the sidecar");
    truncate_wal_records(&wal).unwrap();

    let mut reopened = FilePager::open(&path, true).unwrap();
    assert_eq!(reopened.get_kv(b"alpha").unwrap().as_deref(), Some(b"one".as_slice()));
    reopened.put_kv(b"beta", b"two").unwrap();
    drop(reopened);

    let mut reopened = FilePager::open(&path, true).unwrap();
    assert_eq!(reopened.get_kv(b"alpha").unwrap().as_deref(), Some(b"one".as_slice()));
    assert_eq!(
        reopened.get_kv(b"beta").unwrap().as_deref(),
        Some(b"two".as_slice()),
        "writes appended to an emptied WAL must stay above the checkpoint LSN"
    );
    cleanup(&path);
}

#[test]
fn checkpoint_failure_keeps_committed_wal() {
    let path = temp_db("readonly");
    let mut pager = FilePager::open(&path, true).unwrap();
    pager.put_kv(b"alpha", b"one").unwrap();
    let wal_path = wal_sidecar_path(&path);

    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&path, permissions).unwrap();
    assert!(pager.checkpoint().is_err(), "a write failure must abort the checkpoint");
    assert!(wal_path.exists(), "a failed checkpoint must keep the WAL");
    assert_eq!(checkpoint_lsn(&path), 0);

    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(false);
    std::fs::set_permissions(&path, permissions).unwrap();
    drop(pager);

    let mut reopened = FilePager::open(&path, true).unwrap();
    assert_eq!(reopened.get_kv(b"alpha").unwrap().as_deref(), Some(b"one".as_slice()));
    cleanup(&path);
}

#[test]
fn replay_after_checkpoint_lsn_skips_applied_records() {
    let mut body = Vec::new();
    body.extend_from_slice(&1_u64.to_le_bytes());
    body.extend_from_slice(&9_u32.to_le_bytes());
    body.extend_from_slice(&1_u64.to_le_bytes());
    body.extend_from_slice(&[0x99; PAGE_SIZE]);

    let mut bytes = encode_wal_header(DB_ID, 0);
    bytes.extend_from_slice(&encode_frame(0x01, 1, &1_u64.to_le_bytes()).unwrap());
    bytes.extend_from_slice(&encode_frame(0x02, 2, &body).unwrap());
    bytes.extend_from_slice(&encode_frame(0x04, 3, &[0; 56]).unwrap());
    let wal = WalFile {
        header: WalHeader { database_id: DB_ID, base_checkpoint_lsn: 0, format_version: 0 },
        frames: parse_wal(&bytes).unwrap().frames,
    };

    let sentinel = vec![0x11; PAGE_SIZE];
    let mut pages = BTreeMap::from([(9_u32, sentinel.clone())]);
    replay_wal_pages_after(&wal, 3, &mut pages).unwrap();
    assert_eq!(pages.get(&9), Some(&sentinel), "frames at or below the checkpoint must not replay");

    let mut pages = BTreeMap::new();
    replay_wal_pages(&wal, &mut pages).unwrap();
    assert_eq!(pages.get(&9), Some(&vec![0x99; PAGE_SIZE]));
}
