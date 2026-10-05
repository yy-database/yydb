// gate: format-v1 golden bytes (Living 09)
// fixture: format_v1.*

use std::path::PathBuf;

use yydb_format::{
    committed_transactions, crc32c, parse_blob_header, parse_page0, parse_shm, parse_wal,
    BLOB_HEADER_BYTES,
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/format-v1")
        .join(name)
}

#[test]
fn format_v1_page0_empty() {
    let bytes = std::fs::read(fixture("page0_empty_dual_slot.bin")).unwrap();
    assert_eq!(bytes.len(), 4096);
    let header = parse_page0(&bytes).unwrap();
    assert_eq!(header.slot.generation, 1);
    assert_eq!(header.slot.format_version, 1);
    assert_eq!(header.slot.storage_layout, 0x01);
}

#[test]
fn format_v1_page0_bad_checksum() {
    let bytes = std::fs::read(fixture("page0_bad_checksum.bin")).unwrap();
    let header = parse_page0(&bytes).unwrap();
    assert_eq!(header.slot.slot_kind, 0x42);
}

#[test]
fn format_v1_wal_header() {
    let bytes = std::fs::read(fixture("wal_header_v3.bin")).unwrap();
    let wal = parse_wal(&bytes).unwrap();
    assert_eq!(wal.header.format_version, 1);
    assert!(wal.frames.is_empty());
}

#[test]
fn format_v1_wal_txn_minimal() {
    let bytes = std::fs::read(fixture("wal_txn_commit_minimal.bin")).unwrap();
    let wal = parse_wal(&bytes).unwrap();
    assert_eq!(wal.frames.len(), 3);
    assert_eq!(wal.frames[0].frame_type, 0x01);
    assert_eq!(wal.frames[1].frame_type, 0x02);
    assert_eq!(wal.frames[2].frame_type, 0x04);
    assert_eq!(committed_transactions(&wal), 1);
}

#[test]
fn format_v1_shm_empty() {
    let bytes = std::fs::read(fixture("shm_v2_empty.bin")).unwrap();
    let shm = parse_shm(&bytes).unwrap();
    assert_eq!(shm.format_version, 1);
    assert_eq!(shm.active_readers, 0);
}

#[test]
fn format_v1_blob_header_min() {
    let bytes = std::fs::read(fixture("blob_chunk_header_min.bin")).unwrap();
    assert_eq!(bytes.len(), BLOB_HEADER_BYTES);
    let header = parse_blob_header(&bytes).unwrap();
    assert_eq!(header.chunk_kind, 0x01);
    assert_eq!(header.chunk_len, 0);
}

#[test]
fn format_v1_crc32c_empty() {
    let reference = std::fs::read_to_string(fixture("crc32c_reference.txt")).unwrap();
    assert!(reference.contains("0x00000000"));
    assert_eq!(crc32c(&[]), 0);
}
