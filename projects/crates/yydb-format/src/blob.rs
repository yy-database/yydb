//! `YBLO` v1 immutable chunk header.

use yydb_types::{Error, Result};

pub const BLOB_MAGIC: &[u8; 5] = b"YBLO\x01";
pub const BLOB_HEADER_BYTES: usize = 82;

/// Parsed blob chunk header (payload follows in file).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobChunkHeader {
    pub chunk_kind: u8,
    pub chunk_hash: [u8; 32],
    pub logical_offset: u64,
    pub chunk_len: u32,
    pub payload_hash: [u8; 32],
}

pub fn parse_blob_header(bytes: &[u8]) -> Result<BlobChunkHeader> {
    if bytes.len() < BLOB_HEADER_BYTES {
        return Err(Error::Corrupt("blob header truncated"));
    }
    if bytes.get(0..5) != Some(BLOB_MAGIC.as_slice()) {
        return Err(Error::Corrupt("blob magic mismatch"));
    }
    Ok(BlobChunkHeader {
        chunk_kind: bytes[5],
        chunk_hash: bytes[6..38].try_into().unwrap(),
        logical_offset: u64::from_le_bytes(bytes[38..46].try_into().unwrap()),
        chunk_len: u32::from_le_bytes(bytes[46..50].try_into().unwrap()),
        payload_hash: bytes[50..82].try_into().unwrap(),
    })
}
