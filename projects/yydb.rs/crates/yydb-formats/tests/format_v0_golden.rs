// fixture: format_v0.golden
// fixture: format_v0.*

use std::path::PathBuf;

use yydb_formats::{
    BLOB_HEADER_BYTES, PAGE_FORMAT_VERSION, SLOT_BYTES, committed_tail_lsn, committed_transactions, crc32c,
    parse_blob_header, parse_page0, parse_page0_checked, parse_shm, parse_wal, parse_wal_recover,
    strip_trailing_txn_commit,
};
use yydb_types::Error;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/format-v0").join(name)
}

#[test]
fn format_v0_page0_empty() {
    let bytes = std::fs::read(fixture("page0_empty_dual_slot.bin")).unwrap();
    assert_eq!(bytes.len(), 4096);
    let header = parse_page0(&bytes).unwrap();
    assert_eq!(header.slot.generation, 1);
    assert_eq!(header.slot.format_version, PAGE_FORMAT_VERSION);
    assert_eq!(header.slot.storage_layout, 0x01);
}

#[test]
fn retired_page_format_is_refused() {
    let mut bytes = std::fs::read(fixture("page0_empty_dual_slot.bin")).unwrap();
    for offset in [0, SLOT_BYTES] {
        bytes[offset + 40..offset + 44].copy_from_slice(&0_u32.to_le_bytes());
        let checksum = crc32c(&bytes[offset..offset + 2044]);
        bytes[offset + 2044..offset + 2048].copy_from_slice(&checksum.to_le_bytes());
    }
    let header = parse_page0(&bytes).unwrap();
    assert_eq!(header.slot.format_version, 0);
    let error = parse_page0_checked(&bytes).expect_err("a retired page format must be refused");
    assert!(matches!(error, Error::Unsupported(_)), "unexpected error: {error}");
}

#[test]
fn format_v0_page0_bad_checksum() {
    let bytes = std::fs::read(fixture("page0_bad_checksum.bin")).unwrap();
    let header = parse_page0(&bytes).unwrap();
    assert_eq!(header.slot.slot_kind, 0x42);
}

#[test]
fn format_v0_wal_header() {
    let bytes = std::fs::read(fixture("wal_header.bin")).unwrap();
    let wal = parse_wal(&bytes).unwrap();
    assert_eq!(wal.header.format_version, 0);
    assert!(wal.frames.is_empty());
}

#[test]
fn format_v0_wal_txn_minimal() {
    let bytes = std::fs::read(fixture("wal_txn_commit_minimal.bin")).unwrap();
    let wal = parse_wal(&bytes).unwrap();
    assert_eq!(wal.frames.len(), 3);
    assert_eq!(wal.frames[0].frame_type, 0x01);
    assert_eq!(wal.frames[1].frame_type, 0x02);
    assert_eq!(wal.frames[2].frame_type, 0x04);
    assert_eq!(committed_transactions(&wal), 1);
}

#[test]
fn format_v0_shm_empty() {
    let bytes = std::fs::read(fixture("shm_empty.bin")).unwrap();
    let shm = parse_shm(&bytes).unwrap();
    assert_eq!(shm.format_version, 0);
    assert_eq!(shm.active_readers, 0);
}

#[test]
fn format_v0_blob_header_min() {
    let bytes = std::fs::read(fixture("blob_chunk_header_min.bin")).unwrap();
    assert_eq!(bytes.len(), BLOB_HEADER_BYTES);
    let header = parse_blob_header(&bytes).unwrap();
    assert_eq!(header.chunk_kind, 0x01);
    assert_eq!(header.chunk_len, 0);
}

#[test]
fn format_v0_blob_chunk_roundtrip() {
    use yydb_formats::{encode_blob_chunk, read_blob_payload};
    let payload = b"hello-yblo";
    let file = encode_blob_chunk(payload, 0);
    assert_eq!(read_blob_payload(&file).unwrap(), payload);
}

#[test]
fn format_v0_wal_recover_truncated_tail() {
    let bytes = std::fs::read(fixture("wal_txn_commit_minimal.bin")).unwrap();
    let truncated = &bytes[..bytes.len() - 10];
    assert!(parse_wal(truncated).is_err());
    let wal = parse_wal_recover(truncated).unwrap();
    assert_eq!(wal.frames.len(), 2);
    assert_eq!(committed_transactions(&wal), 0);
}

#[test]
fn format_v0_committed_tail_lsn() {
    let bytes = std::fs::read(fixture("wal_txn_commit_minimal.bin")).unwrap();
    let wal = parse_wal(&bytes).unwrap();
    assert_eq!(committed_tail_lsn(&wal), wal.frames[2].frame_lsn);
}

#[test]
fn format_v0_strip_trailing_commit() {
    let bytes = std::fs::read(fixture("wal_txn_commit_minimal.bin")).unwrap();
    assert_eq!(committed_transactions(&parse_wal(&bytes).unwrap()), 1);
    let stripped = strip_trailing_txn_commit(&bytes).unwrap();
    let wal = parse_wal(&stripped).unwrap();
    assert!(wal.frames.iter().all(|frame| frame.frame_type != 0x04), "stripped wal still has commit frames: {:?}", wal.frames);
    assert_eq!(wal.frames.len(), 2);
}

#[test]
fn format_v0_crc32c_empty() {
    let reference = std::fs::read_to_string(fixture("crc32c_reference.txt")).unwrap();
    assert!(reference.contains("0x00000000"));
    assert_eq!(crc32c(&[]), 0);
}
