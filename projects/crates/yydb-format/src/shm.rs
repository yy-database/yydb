//! `YYSH` v0 shared-memory coordination block.

use yydb_types::{Error, Result};

use crate::crc32c::crc32c;

pub const SHM_MAGIC: &[u8; 5] = b"YYSH\x00";
pub const SHM_BYTES: usize = 4096;

/// Parsed SHM v0 block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShmBlock {
    pub format_version: u32,
    pub writer_epoch: u64,
    pub active_readers: u32,
    pub wal_tail_lsn: u64,
    pub wal_file_bytes: u64,
}

pub fn parse_shm(bytes: &[u8]) -> Result<ShmBlock> {
    if bytes.len() < SHM_BYTES {
        return Err(Error::Corrupt("shm truncated"));
    }
    if bytes.get(0..5) != Some(SHM_MAGIC.as_slice()) {
        return Err(Error::Corrupt("shm magic mismatch"));
    }
    let stored = u32::from_le_bytes(bytes[4092..4096].try_into().unwrap());
    if crc32c(&bytes[..4092]) != stored {
        return Err(Error::Corrupt("shm checksum mismatch"));
    }
    Ok(ShmBlock {
        format_version: u32::from_le_bytes(bytes[5..9].try_into().unwrap()),
        writer_epoch: u64::from_le_bytes(bytes[9..17].try_into().unwrap()),
        active_readers: u32::from_le_bytes(bytes[17..21].try_into().unwrap()),
        wal_tail_lsn: u64::from_le_bytes(bytes[21..29].try_into().unwrap()),
        wal_file_bytes: u64::from_le_bytes(bytes[29..37].try_into().unwrap()),
    })
}

pub fn encode_empty_shm() -> Vec<u8> {
    let mut buf = vec![0_u8; SHM_BYTES];
    buf[0..5].copy_from_slice(SHM_MAGIC);
    buf[5..9].copy_from_slice(&0u32.to_le_bytes());
    let checksum = crc32c(&buf[..4092]);
    buf[4092..4096].copy_from_slice(&checksum.to_le_bytes());
    buf
}
