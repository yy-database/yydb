//! fixture: format_v0.checkpoint_header_crash

use std::fs;

use yydb::{journal::wal_path, Connection, OpenFlags};
use yydb_format::{inactive_slot_offset, parse_page0, slot_offset_for_kind, FilePager};

fn temp_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "yydb-format-v0-checkpoint-header-crash-{}-{}.yydb",
        std::process::id(),
        label
    ))
}

fn prepare_wal_pending(path: &std::path::Path) {
    let wal = wal_path(path);
    let _ = fs::remove_file(&wal);
    let _ = fs::remove_file(path);

    let conn = Connection::open_with_flags(path, OpenFlags::wal()).unwrap();
    conn.put("baseline", b"ok").unwrap();
    conn.checkpoint().unwrap();
    assert!(!wal.exists(), "baseline checkpoint must truncate WAL");
    conn.put("pending", b"wal").unwrap();
    drop(conn);
    assert!(wal.exists(), "pending commit must leave WAL sidecar");
}

#[test]
fn yydb_format_v0_checkpoint_header_crash_before_commit() {
    let path = temp_path("before-header");
    prepare_wal_pending(&path);
    let wal = wal_path(&path);

    {
        let mut pager = FilePager::open(&path, true).unwrap();
        pager.simulate_checkpoint_crash_before_header_commit().unwrap();
    }

    assert!(
        wal.exists(),
        "checkpoint must not truncate WAL before page 0 header commit"
    );

    let reopened = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    assert_eq!(reopened.get("baseline").unwrap(), Some(b"ok".to_vec()));
    assert_eq!(
        reopened.get("pending").unwrap(),
        Some(b"wal".to_vec()),
        "fixture format_v0.checkpoint_header_crash WAL must recover pending key"
    );

    let _ = fs::remove_file(&wal);
    let _ = fs::remove_file(&path);
}

#[test]
fn yydb_format_v0_checkpoint_header_crash_corrupt_inactive_slot() {
    let path = temp_path("corrupt-inactive");
    prepare_wal_pending(&path);
    let wal = wal_path(&path);

    let prior_header = parse_page0(&fs::read(&path).unwrap()).unwrap();
    let prior_kind = prior_header.slot.slot_kind;
    let prior_generation = prior_header.slot.generation;

    {
        let mut pager = FilePager::open(&path, true).unwrap();
        pager
            .simulate_checkpoint_crash_corrupt_inactive_header()
            .unwrap();
    }

    let bytes = fs::read(&path).unwrap();
    let header = parse_page0(&bytes).unwrap();
    assert_eq!(
        header.slot.slot_kind,
        prior_kind,
        "fixture format_v0.checkpoint_header_crash must keep prior active header slot"
    );
    assert_eq!(
        header.slot.generation,
        prior_generation,
        "fixture format_v0.checkpoint_header_crash must not advance generation on torn header write"
    );
    let inactive_offset =
        inactive_slot_offset(slot_offset_for_kind(prior_kind));
    let inactive_generation =
        u64::from_le_bytes(bytes[inactive_offset + 6..inactive_offset + 14].try_into().unwrap());
    assert!(
        inactive_generation > header.slot.generation,
        "inactive slot may carry a torn generation but must not win"
    );

    assert!(
        wal.exists(),
        "corrupt header commit must not truncate WAL"
    );

    let reopened = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    assert_eq!(reopened.get("baseline").unwrap(), Some(b"ok".to_vec()));
    assert_eq!(
        reopened.get("pending").unwrap(),
        Some(b"wal".to_vec()),
        "fixture format_v0.checkpoint_header_crash WAL must recover pending key"
    );

    let _ = fs::remove_file(&wal);
    let _ = fs::remove_file(&path);
}
