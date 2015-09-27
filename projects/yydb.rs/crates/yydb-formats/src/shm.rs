//! `YYSH` v0 shared-memory coordination block.

use std::path::{Path, PathBuf};

use yydb_types::{Error, Result};

use crate::{
    crc32c::crc32c,
    wal::{committed_tail_lsn, parse_wal_recover},
};

/// Five-byte magic prefix for SHM sidecar files.
pub const SHM_MAGIC: &[u8; 5] = b"YYSH\x00";
/// Fixed SHM block size for v0.
pub const SHM_BYTES: usize = 4096;

/// Parsed SHM v0 block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShmBlock {
    /// SHM format version (`0` for v0).
    pub format_version: u32,
    /// Writer epoch for lock coordination.
    pub writer_epoch: u64,
    /// Active reader count.
    pub active_readers: u32,
    /// Tail LSN mirrored from WAL.
    pub wal_tail_lsn: u64,
    /// WAL file length in bytes.
    pub wal_file_bytes: u64,
}

/// Parse and validate a v0 SHM block.
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

/// Encode a checksum-valid SHM block.
pub fn encode_shm(block: &ShmBlock) -> Vec<u8> {
    let mut buf = vec![0_u8; SHM_BYTES];
    buf[0..5].copy_from_slice(SHM_MAGIC);
    buf[5..9].copy_from_slice(&block.format_version.to_le_bytes());
    buf[9..17].copy_from_slice(&block.writer_epoch.to_le_bytes());
    buf[17..21].copy_from_slice(&block.active_readers.to_le_bytes());
    buf[21..29].copy_from_slice(&block.wal_tail_lsn.to_le_bytes());
    buf[29..37].copy_from_slice(&block.wal_file_bytes.to_le_bytes());
    let checksum = crc32c(&buf[..4092]);
    buf[4092..4096].copy_from_slice(&checksum.to_le_bytes());
    buf
}

/// Encode an empty, checksum-valid SHM block for golden fixtures.
pub fn encode_empty_shm() -> Vec<u8> {
    encode_shm(&ShmBlock { format_version: 0, writer_epoch: 0, active_readers: 0, wal_tail_lsn: 0, wal_file_bytes: 0 })
}

/// Resolve `<main>-shm` for a database main file path.
pub fn shm_sidecar_path(main: &Path) -> PathBuf {
    let mut sidecar = main.as_os_str().to_owned();
    sidecar.push("-shm");
    PathBuf::from(sidecar)
}

/// Mirror committed WAL tail metadata into the `-shm` sidecar.
///
/// This path re-parses the whole WAL, so callers that already appended a
/// committed transaction and know its tail should use
/// [`sync_shm_from_commit`] instead.
pub fn sync_shm_from_wal(main: &Path, wal: &Path) -> Result<()> {
    let wal_bytes = std::fs::read(wal)?;
    let parsed = parse_wal_recover(&wal_bytes)?;
    write_shm(main, committed_tail_lsn(&parsed), wal_bytes.len() as u64)
}

/// Mirror a WAL tail that the writer already reported into the `-shm` sidecar.
///
/// `tail_lsn` must come from a successful append and `wal_file_bytes` must be
/// the sidecar length after that append, so the block is byte-identical to
/// what [`sync_shm_from_wal`] would derive from a fresh parse.
pub fn sync_shm_from_commit(main: &Path, tail_lsn: u64, wal_file_bytes: u64) -> Result<()> {
    write_shm(main, tail_lsn, wal_file_bytes)
}

fn write_shm(main: &Path, tail_lsn: u64, wal_file_bytes: u64) -> Result<()> {
    let block = ShmBlock { format_version: 0, writer_epoch: 0, active_readers: 0, wal_tail_lsn: tail_lsn, wal_file_bytes };
    std::fs::write(shm_sidecar_path(main), encode_shm(&block))?;
    Ok(())
}
