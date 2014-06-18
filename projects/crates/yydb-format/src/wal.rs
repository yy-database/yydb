//! `YYWL` v0 page-image WAL.

use yydb_types::{Error, Result};

use crate::crc32c::crc32c;

/// Five-byte magic prefix for WAL sidecar files.
pub const WAL_MAGIC: &[u8; 5] = b"YYWL\x00";

/// Parsed WAL file header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalHeader {
    /// Database id copied from the main file header.
    pub database_id: [u8; 16],
    /// Checkpoint LSN this WAL file continues from.
    pub base_checkpoint_lsn: u64,
    /// WAL format version (`0` for v0).
    pub format_version: u32,
}

/// One validated WAL frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalFrame {
    /// Frame type discriminator.
    pub frame_type: u8,
    /// Monotonic frame LSN.
    pub frame_lsn: u64,
    /// Frame body bytes (type-specific).
    pub body: Vec<u8>,
}

/// Full WAL scan result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalFile {
    /// Parsed file header.
    pub header: WalHeader,
    /// Frames in file order.
    pub frames: Vec<WalFrame>,
}

/// Parse and validate the WAL file header; returns header and byte length consumed.
pub fn parse_header(bytes: &[u8]) -> Result<(WalHeader, usize)> {
    if bytes.len() < 37 {
        return Err(Error::Corrupt("wal header truncated"));
    }
    if bytes.get(0..5) != Some(WAL_MAGIC.as_slice()) {
        return Err(Error::Corrupt("wal magic mismatch"));
    }
    let stored = u32::from_le_bytes(bytes[33..37].try_into().unwrap());
    let computed = crc32c(&bytes[..33]);
    if stored != computed {
        return Err(Error::Corrupt("wal header checksum mismatch"));
    }
    Ok((
        WalHeader {
            database_id: bytes[5..21].try_into().unwrap(),
            base_checkpoint_lsn: u64::from_le_bytes(bytes[21..29].try_into().unwrap()),
            format_version: u32::from_le_bytes(bytes[29..33].try_into().unwrap()),
        },
        37,
    ))
}

fn parse_frame(bytes: &[u8], offset: usize) -> Result<(WalFrame, usize)> {
    if bytes.len() < offset + 13 {
        return Err(Error::Corrupt("wal frame truncated"));
    }
    let frame_type = bytes[offset];
    let frame_lsn = u64::from_le_bytes(bytes[offset + 1..offset + 9].try_into().unwrap());
    let frame_len = u32::from_le_bytes(bytes[offset + 9..offset + 13].try_into().unwrap()) as usize;
    if frame_len < 9 {
        return Err(Error::Corrupt("wal frame length invalid"));
    }
    let body_len = frame_len - 9;
    let body_start = offset + 13;
    let body_end = body_start + body_len;
    let checksum_end = body_end + 4;
    if bytes.len() < checksum_end {
        return Err(Error::Corrupt("wal frame body truncated"));
    }
    let frame_without_checksum = &bytes[offset..body_end];
    let stored_checksum = u32::from_le_bytes(bytes[body_end..checksum_end].try_into().unwrap());
    if crc32c(frame_without_checksum) != stored_checksum {
        return Err(Error::Corrupt("wal frame checksum mismatch"));
    }
    Ok((
        WalFrame {
            frame_type,
            frame_lsn,
            body: bytes[body_start..body_end].to_vec(),
        },
        checksum_end - offset,
    ))
}

/// Parse a complete WAL file.
pub fn parse_wal(bytes: &[u8]) -> Result<WalFile> {
    let (header, mut offset) = parse_header(bytes)?;
    let mut frames = Vec::new();
    while offset < bytes.len() {
        let (frame, consumed) = parse_frame(bytes, offset)?;
        frames.push(frame);
        offset += consumed;
    }
    Ok(WalFile { header, frames })
}

/// Parse WAL bytes for recovery, dropping an incomplete trailing frame.
pub fn parse_wal_recover(bytes: &[u8]) -> Result<WalFile> {
    let (header, mut offset) = parse_header(bytes)?;
    let mut frames = Vec::new();
    while offset < bytes.len() {
        match parse_frame(bytes, offset) {
            Ok((frame, consumed)) => {
                frames.push(frame);
                offset += consumed;
            }
            Err(_) => break,
        }
    }
    Ok(WalFile { header, frames })
}

/// Highest `frame_lsn` among committed transactions in `wal`.
pub fn committed_tail_lsn(wal: &WalFile) -> u64 {
    wal.frames
        .iter()
        .filter(|frame| frame.frame_type == 0x04)
        .map(|frame| frame.frame_lsn)
        .max()
        .unwrap_or(0)
}

/// Byte offsets of each frame start after the WAL header.
pub fn wal_frame_offsets(bytes: &[u8]) -> Result<Vec<usize>> {
    let (_, mut offset) = parse_header(bytes)?;
    let mut offsets = Vec::new();
    while offset < bytes.len() {
        offsets.push(offset);
        let (_, consumed) = parse_frame(bytes, offset)?;
        offset += consumed;
    }
    Ok(offsets)
}

/// Return committed transaction count (`TxnCommit` frames with valid prefixes).
pub fn committed_transactions(wal: &WalFile) -> usize {
    wal.frames
        .iter()
        .filter(|frame| frame.frame_type == 0x04)
        .count()
}
