// gate: format-v1 (Living 09)
// fixture: format_v1.*

use std::path::PathBuf;

use yydb_format::{
    committed_transactions, crc32c, parse_blob_header, parse_page0, parse_shm, parse_wal,
    BLOB_HEADER_BYTES,
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../yydb-format/tests/fixtures/format-v1")
        .join(name)
}

#[test]
fn g_yydb_format_v1_golden_page0_empty() {
    let bytes = std::fs::read(fixture("page0_empty_dual_slot.bin")).unwrap();
    let header = parse_page0(&bytes).unwrap();
    assert_eq!(header.slot.generation, 1);
}

#[test]
fn g_yydb_format_v1_golden_page0_bad_checksum() {
    let bytes = std::fs::read(fixture("page0_bad_checksum.bin")).unwrap();
    let header = parse_page0(&bytes).unwrap();
    assert_eq!(header.slot.slot_kind, 0x42);
}

#[test]
fn g_yydb_format_v1_golden_wal_txn_minimal() {
    let bytes = std::fs::read(fixture("wal_txn_commit_minimal.bin")).unwrap();
    let wal = parse_wal(&bytes).unwrap();
    assert_eq!(committed_transactions(&wal), 1);
}

#[test]
fn g_yydb_format_v1_golden_blob_header() {
    let bytes = std::fs::read(fixture("blob_chunk_header_min.bin")).unwrap();
    assert_eq!(bytes.len(), BLOB_HEADER_BYTES);
    parse_blob_header(&bytes).unwrap();
}

#[test]
fn g_yydb_format_v1_golden_shm_empty() {
    parse_shm(&std::fs::read(fixture("shm_v2_empty.bin")).unwrap()).unwrap();
}

#[test]
fn g_yydb_format_v1_golden_crc32c() {
    assert_eq!(crc32c(&[]), 0);
}
