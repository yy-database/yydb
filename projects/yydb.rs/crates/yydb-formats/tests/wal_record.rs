//! `YYWL` v0 record validation and torn-tail classification.

use yydb_formats::{
    MAX_WAL_RECORD_BYTES, WAL_HEADER_BYTES, WAL_RECORD_HEADER_BYTES, WalRecordError, WalTail, crc32c, encode_frame,
    encode_wal_header, parse_wal, parse_wal_recover, scan_wal, truncate_wal_to_recoverable_prefix, wal_frame_offsets,
    wal_recoverable_byte_len,
};

const DB_ID: [u8; 16] = [0x11; 16];

fn wal_bytes() -> Vec<u8> {
    let mut bytes = encode_wal_header(DB_ID, 0);
    bytes.extend_from_slice(&encode_frame(0x01, 1, &[0xAA]).unwrap());
    bytes.extend_from_slice(&encode_frame(0x02, 2, &[0xBB; 20]).unwrap());
    bytes.extend_from_slice(&encode_frame(0x04, 3, &[0xCC; 56]).unwrap());
    bytes
}

fn record_len(bytes: &[u8], offset: usize) -> usize {
    let declared = u32::from_le_bytes(bytes[offset + 9..offset + 13].try_into().unwrap()) as usize;
    declared + 8
}

#[test]
fn wal_records_append_in_order() {
    let bytes = wal_bytes();
    let scan = scan_wal(&bytes).unwrap();
    assert_eq!(scan.header.database_id, DB_ID);
    assert_eq!(scan.header.base_checkpoint_lsn, 0);
    assert_eq!(scan.header.format_version, 0);
    assert_eq!(scan.tail, WalTail::Clean);
    assert_eq!(scan.valid_len, bytes.len());
    assert_eq!(scan.frames.len(), 3);
    assert_eq!(scan.frames.iter().map(|frame| frame.frame_lsn).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert_eq!(scan.frames.iter().map(|frame| frame.frame_type).collect::<Vec<_>>(), vec![1, 2, 4]);
    assert_eq!(scan.frames[1].body.len(), 20);
}

#[test]
fn wal_every_torn_prefix_is_classified() {
    let bytes = wal_bytes();
    let offsets = wal_frame_offsets(&bytes).unwrap();
    assert_eq!(offsets.len(), 3);
    assert_eq!(offsets[0], WAL_HEADER_BYTES);

    for cut in WAL_HEADER_BYTES..bytes.len() {
        let scan = scan_wal(&bytes[..cut]).unwrap_or_else(|error| panic!("cut at {cut} reported corruption: {error}"));
        if offsets.contains(&cut) {
            assert_eq!(scan.tail, WalTail::Clean, "cut at record boundary {cut} must be clean");
            assert_eq!(scan.valid_len, cut);
        }
        else {
            match scan.tail {
                WalTail::Torn { offset, available, .. } => {
                    let start = offsets.iter().rev().find(|start| **start <= cut).copied().unwrap();
                    assert_eq!(offset, start, "torn record must start at the last record boundary");
                    assert_eq!(available, cut - start);
                    assert_eq!(scan.valid_len, start);
                }
                WalTail::Clean => panic!("cut at {cut} inside a record must not be clean"),
            }
        }
    }
}

#[test]
fn wal_torn_tail_truncates_to_last_complete_record() {
    let bytes = wal_bytes();
    let offsets = wal_frame_offsets(&bytes).unwrap();
    let cut = offsets[2] + record_len(&bytes, offsets[2]) - 3;

    assert!(parse_wal(&bytes[..cut]).is_err(), "full parse must reject a torn tail");
    let recovered = parse_wal_recover(&bytes[..cut]).unwrap();
    assert_eq!(recovered.frames.len(), 2);
    assert_eq!(wal_recoverable_byte_len(&bytes[..cut]).unwrap(), offsets[2]);
    assert_eq!(truncate_wal_to_recoverable_prefix(&bytes[..cut]).unwrap(), bytes[..offsets[2]].to_vec());
}

#[test]
fn wal_checksum_corruption_is_not_a_torn_tail() {
    let bytes = wal_bytes();
    let offsets = wal_frame_offsets(&bytes).unwrap();
    let mut corrupted = bytes.clone();
    corrupted[offsets[1] + WAL_RECORD_HEADER_BYTES + 4] ^= 0xFF;

    assert_eq!(
        scan_wal(&corrupted).unwrap_err(),
        WalRecordError::RecordChecksumMismatch { offset: offsets[1] },
        "a complete record with a bad CRC must be reported as corruption"
    );
    assert!(parse_wal_recover(&corrupted).is_err());
    assert!(wal_recoverable_byte_len(&corrupted).is_err());
    assert!(truncate_wal_to_recoverable_prefix(&corrupted).is_err());
}

#[test]
fn wal_overlong_length_is_corruption() {
    let bytes = wal_bytes();
    let offsets = wal_frame_offsets(&bytes).unwrap();
    let mut overlong = bytes.clone();
    overlong[offsets[0] + 9..offsets[0] + 13].copy_from_slice(&(MAX_WAL_RECORD_BYTES as u32 + 1).to_le_bytes());

    assert_eq!(
        scan_wal(&overlong).unwrap_err(),
        WalRecordError::RecordLengthOutOfRange { offset: offsets[0], declared: MAX_WAL_RECORD_BYTES as u64 + 1 }
    );

    let mut underlong = bytes.clone();
    underlong[offsets[0] + 9..offsets[0] + 13].copy_from_slice(&8_u32.to_le_bytes());
    assert_eq!(scan_wal(&underlong).unwrap_err(), WalRecordError::RecordLengthOutOfRange { offset: offsets[0], declared: 8 });
}

#[test]
fn wal_out_of_order_lsn_is_corruption() {
    let mut bytes = encode_wal_header(DB_ID, 0);
    bytes.extend_from_slice(&encode_frame(0x01, 5, &[1]).unwrap());
    bytes.extend_from_slice(&encode_frame(0x02, 5, &[2]).unwrap());

    assert_eq!(
        scan_wal(&bytes).unwrap_err(),
        WalRecordError::RecordLsnOutOfOrder {
            offset: WAL_HEADER_BYTES + record_len(&bytes, WAL_HEADER_BYTES),
            previous: 5,
            found: 5
        }
    );
}

#[test]
fn wal_header_failures_are_structured() {
    let bytes = wal_bytes();
    assert_eq!(
        scan_wal(&bytes[..10]).unwrap_err(),
        WalRecordError::HeaderTruncated { available: 10, required: WAL_HEADER_BYTES }
    );

    let mut bad_magic = bytes.clone();
    bad_magic[0] = b'X';
    assert_eq!(scan_wal(&bad_magic).unwrap_err(), WalRecordError::MagicMismatch);

    let mut bad_version = bytes.clone();
    bad_version[29..33].copy_from_slice(&1_u32.to_le_bytes());
    let checksum = crc32c(&bad_version[..33]);
    bad_version[33..37].copy_from_slice(&checksum.to_le_bytes());
    assert_eq!(scan_wal(&bad_version).unwrap_err(), WalRecordError::UnsupportedVersion { found: 1 });

    let mut bad_checksum = bytes;
    bad_checksum[33] ^= 0xFF;
    assert_eq!(scan_wal(&bad_checksum).unwrap_err(), WalRecordError::HeaderChecksumMismatch);
}
