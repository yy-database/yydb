//! fixture: format_v0.checkpoint_header

use std::fs;

use yydb::{Connection, OpenFlags};
use yydb_format::{parse_page0, SLOT_BYTES, SLOT_KIND_A, SLOT_KIND_B};

fn temp_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "yydb-format-v0-checkpoint-header-{}-{}.yydb",
        std::process::id(),
        label
    ))
}

#[test]
fn yydb_format_v0_checkpoint_header() {
    let path = temp_path("checkpoint");
    let _ = fs::remove_file(&path);

    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    conn.put("k", b"v").unwrap();
    conn.checkpoint().unwrap();
    drop(conn);

    let bytes = fs::read(&path).unwrap();
    let header = parse_page0(&bytes).unwrap();
    assert!(header.slot.generation > 1);
    assert!(header.slot.checkpoint_lsn > 0);

    let _ = fs::remove_file(&path);
}

fn slot_kind(bytes: &[u8], offset: usize) -> u8 {
    bytes[offset + 5]
}

fn slot_generation(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset + 6..offset + 14].try_into().unwrap())
}

#[test]
fn yydb_format_v0_checkpoint_alternates_header_slot() {
    let path = temp_path("alternate");
    let _ = fs::remove_file(&path);

    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    conn.put("k1", b"v1").unwrap();
    conn.checkpoint().unwrap();
    let bytes = fs::read(&path).unwrap();
    let header = parse_page0(&bytes).unwrap();
    assert_eq!(header.slot.slot_kind, SLOT_KIND_B);
    assert_eq!(slot_generation(&bytes, 0), 1);
    assert_eq!(slot_generation(&bytes, SLOT_BYTES), 2);

    conn.put("k2", b"v2").unwrap();
    conn.checkpoint().unwrap();
    drop(conn);

    let bytes = fs::read(&path).unwrap();
    let header = parse_page0(&bytes).unwrap();
    assert_eq!(header.slot.slot_kind, SLOT_KIND_A);
    assert_eq!(slot_generation(&bytes, 0), 3);
    assert_eq!(slot_generation(&bytes, SLOT_BYTES), 2);
    assert_eq!(slot_kind(&bytes, SLOT_BYTES), SLOT_KIND_B);

    let _ = fs::remove_file(&path);
}
