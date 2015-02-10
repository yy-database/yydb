//! `YYWL` v0 page-image WAL.

use std::fmt;

use yydb_types::{Error, Result};

use crate::crc32c::crc32c;

/// Five-byte magic prefix for WAL sidecar files.
pub const WAL_MAGIC: &[u8; 5] = b"YYWL\x00";
/// Fixed WAL file header length.
pub const WAL_HEADER_BYTES: usize = 37;
/// Fixed per-record header length (`kind` + `lsn` + `length`).
pub const WAL_RECORD_HEADER_BYTES: usize = 13;
/// Fixed per-record checksum trailer length.
pub const WAL_RECORD_TRAILER_BYTES: usize = 4;
/// Bytes counted by a record length field before the body (`kind` + `lsn`).
const WAL_RECORD_FIXED_BYTES: usize = 9;
/// Bytes outside a record body other than the length field itself.
const WAL_RECORD_LENGTH_AND_TRAILER_BYTES: usize = 8;
/// Largest declared record length accepted from a v0 WAL file.
///
/// The limit keeps a corrupt length field from being mistaken for a torn tail
/// and bounds the memory a recovery scan can allocate.
pub const MAX_WAL_RECORD_BYTES: usize = 8 * 1024 * 1024;

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

/// Structured reason a WAL header or record failed validation.
///
/// Torn tails are represented by [`WalRecordError::RecordTruncated`] and are
/// recoverable by truncation. Every other variant means the bytes on disk are
/// damaged and recovery must fail instead of silently skipping records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalRecordError {
    /// The WAL file is shorter than its fixed header.
    HeaderTruncated {
        /// Bytes present in the file.
        available: usize,
        /// Bytes required by the v0 header.
        required: usize,
    },
    /// The file does not start with [`WAL_MAGIC`].
    MagicMismatch,
    /// The header declares a format version this build does not parse.
    UnsupportedVersion {
        /// Version found in the header.
        found: u32,
    },
    /// The header checksum trailer does not match its bytes.
    HeaderChecksumMismatch,
    /// A record ends before its declared length is available.
    RecordTruncated {
        /// Byte offset of the record start.
        offset: usize,
        /// Record bytes still present.
        available: usize,
        /// Record bytes required by the declared length.
        required: usize,
    },
    /// A record declares a length outside `[9, MAX_WAL_RECORD_BYTES]`.
    RecordLengthOutOfRange {
        /// Byte offset of the record start.
        offset: usize,
        /// Declared record length.
        declared: u64,
    },
    /// A complete record does not match its CRC32C trailer.
    RecordChecksumMismatch {
        /// Byte offset of the record start.
        offset: usize,
    },
    /// A record LSN does not advance beyond the previous record.
    RecordLsnOutOfOrder {
        /// Byte offset of the record start.
        offset: usize,
        /// LSN of the previous record.
        previous: u64,
        /// LSN found in this record.
        found: u64,
    },
}

impl WalRecordError {
    /// Stable diagnostic code for this failure.
    pub fn code(&self) -> &'static str {
        match self {
            Self::HeaderTruncated { .. } => "yydb.wal.header_truncated",
            Self::MagicMismatch => "yydb.wal.magic_mismatch",
            Self::UnsupportedVersion { .. } => "yydb.wal.unsupported_version",
            Self::HeaderChecksumMismatch => "yydb.wal.header_checksum_mismatch",
            Self::RecordTruncated { .. } => "yydb.wal.record_truncated",
            Self::RecordLengthOutOfRange { .. } => "yydb.wal.record_length_out_of_range",
            Self::RecordChecksumMismatch { .. } => "yydb.wal.record_checksum_mismatch",
            Self::RecordLsnOutOfOrder { .. } => "yydb.wal.record_lsn_out_of_order",
        }
    }

    /// Short static description used by [`yydb_types::Error::Corrupt`].
    pub fn summary(&self) -> &'static str {
        match self {
            Self::HeaderTruncated { .. } => "wal header truncated",
            Self::MagicMismatch => "wal magic mismatch",
            Self::UnsupportedVersion { .. } => "wal version unsupported",
            Self::HeaderChecksumMismatch => "wal header checksum mismatch",
            Self::RecordTruncated { .. } => "wal record truncated",
            Self::RecordLengthOutOfRange { .. } => "wal record length out of range",
            Self::RecordChecksumMismatch { .. } => "wal record checksum mismatch",
            Self::RecordLsnOutOfOrder { .. } => "wal record lsn out of order",
        }
    }

    /// True when the bytes are only an incomplete record prefix.
    ///
    /// Only this variant can be truncated by recovery. Checksum and length
    /// failures keep the file damaged.
    pub fn is_torn_tail(&self) -> bool {
        matches!(self, Self::RecordTruncated { .. })
    }
}

impl fmt::Display for WalRecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HeaderTruncated { available, required } => {
                write!(f, "wal header truncated: {available} of {required} bytes")
            }
            Self::MagicMismatch => f.write_str("wal magic mismatch"),
            Self::UnsupportedVersion { found } => write!(f, "wal version {found} is unsupported"),
            Self::HeaderChecksumMismatch => f.write_str("wal header checksum mismatch"),
            Self::RecordTruncated { offset, available, required } => {
                write!(f, "wal record at {offset} truncated: {available} of {required} bytes")
            }
            Self::RecordLengthOutOfRange { offset, declared } => {
                write!(f, "wal record at {offset} declares {declared} bytes")
            }
            Self::RecordChecksumMismatch { offset } => write!(f, "wal record at {offset} checksum mismatch"),
            Self::RecordLsnOutOfOrder { offset, previous, found } => {
                write!(f, "wal record at {offset} has lsn {found} after {previous}")
            }
        }
    }
}

impl std::error::Error for WalRecordError {}

impl From<WalRecordError> for Error {
    fn from(error: WalRecordError) -> Self {
        Self::Corrupt(error.summary())
    }
}

/// Classification of the bytes after the last complete WAL record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalTail {
    /// The file ends on an exact record boundary.
    Clean,
    /// The remaining bytes are an incomplete record prefix and may be truncated.
    Torn {
        /// Byte offset where the incomplete record starts.
        offset: usize,
        /// Bytes still present.
        available: usize,
        /// Record bytes required by the declared length, when the record header was complete.
        required: Option<usize>,
    },
}

/// One recovery-oriented WAL scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalScan {
    /// Parsed file header.
    pub header: WalHeader,
    /// Complete, checksum-valid records in file order.
    pub frames: Vec<WalFrame>,
    /// Bytes after the last complete record.
    pub tail: WalTail,
    /// Header plus complete records. Truncating to this length drops a torn tail.
    pub valid_len: usize,
}

/// Parse and validate the WAL file header; returns header and byte length consumed.
pub fn parse_header(bytes: &[u8]) -> std::result::Result<(WalHeader, usize), WalRecordError> {
    if bytes.len() < WAL_HEADER_BYTES {
        return Err(WalRecordError::HeaderTruncated { available: bytes.len(), required: WAL_HEADER_BYTES });
    }
    if bytes.get(0..5) != Some(WAL_MAGIC.as_slice()) {
        return Err(WalRecordError::MagicMismatch);
    }
    let stored = u32::from_le_bytes(bytes[33..37].try_into().unwrap());
    let computed = crc32c(&bytes[..33]);
    if stored != computed {
        return Err(WalRecordError::HeaderChecksumMismatch);
    }
    let header = WalHeader {
        database_id: bytes[5..21].try_into().unwrap(),
        base_checkpoint_lsn: u64::from_le_bytes(bytes[21..29].try_into().unwrap()),
        format_version: u32::from_le_bytes(bytes[29..33].try_into().unwrap()),
    };
    if header.format_version != 0 {
        return Err(WalRecordError::UnsupportedVersion { found: header.format_version });
    }
    Ok((header, WAL_HEADER_BYTES))
}

/// Parse one WAL frame at `offset`; returns the frame and total byte length consumed.
pub fn parse_frame(bytes: &[u8], offset: usize) -> std::result::Result<(WalFrame, usize), WalRecordError> {
    parse_frame_after(bytes, offset, None)
}

fn parse_frame_after(
    bytes: &[u8],
    offset: usize,
    previous_lsn: Option<u64>,
) -> std::result::Result<(WalFrame, usize), WalRecordError> {
    let available = bytes.len().saturating_sub(offset);
    if available < WAL_RECORD_HEADER_BYTES {
        return Err(WalRecordError::RecordTruncated { offset, available, required: WAL_RECORD_HEADER_BYTES });
    }
    let frame_type = bytes[offset];
    let frame_lsn = u64::from_le_bytes(bytes[offset + 1..offset + 9].try_into().unwrap());
    let frame_len = u32::from_le_bytes(bytes[offset + 9..offset + 13].try_into().unwrap()) as usize;
    // `frame_len` counts the bytes after the four-byte length field: the kind
    // byte, the LSN and the body. It never includes the twelve bytes before it
    // or the four checksum trailer bytes.
    if !(WAL_RECORD_FIXED_BYTES..=MAX_WAL_RECORD_BYTES).contains(&frame_len) {
        return Err(WalRecordError::RecordLengthOutOfRange { offset, declared: frame_len as u64 });
    }
    let body_len = frame_len - WAL_RECORD_FIXED_BYTES;
    let body_start = offset + WAL_RECORD_HEADER_BYTES;
    let body_end = body_start + body_len;
    let record_end = body_end + WAL_RECORD_TRAILER_BYTES;
    if bytes.len() < record_end {
        return Err(WalRecordError::RecordTruncated {
            offset,
            available,
            required: frame_len + WAL_RECORD_LENGTH_AND_TRAILER_BYTES,
        });
    }
    let stored_checksum = u32::from_le_bytes(bytes[body_end..record_end].try_into().unwrap());
    if crc32c(&bytes[offset..body_end]) != stored_checksum {
        return Err(WalRecordError::RecordChecksumMismatch { offset });
    }
    if let Some(previous) = previous_lsn {
        if frame_lsn <= previous {
            return Err(WalRecordError::RecordLsnOutOfOrder { offset, previous, found: frame_lsn });
        }
    }
    Ok((WalFrame { frame_type, frame_lsn, body: bytes[body_start..body_end].to_vec() }, record_end - offset))
}

/// Scan a WAL file, classifying trailing bytes instead of skipping damaged records.
///
/// A torn tail is reported through [`WalScan::tail`]. A checksum, length or LSN
/// failure is returned as [`WalRecordError`] so callers cannot mistake corruption
/// for an incomplete append.
pub fn scan_wal(bytes: &[u8]) -> std::result::Result<WalScan, WalRecordError> {
    let (header, mut offset) = parse_header(bytes)?;
    let mut frames = Vec::new();
    let mut previous_lsn = None;
    while offset < bytes.len() {
        match parse_frame_after(bytes, offset, previous_lsn) {
            Ok((frame, consumed)) => {
                previous_lsn = Some(frame.frame_lsn);
                frames.push(frame);
                offset += consumed;
            }
            Err(WalRecordError::RecordTruncated { .. }) => {
                let available = bytes.len() - offset;
                let required = torn_required_len(bytes, offset);
                return Ok(WalScan { header, frames, tail: WalTail::Torn { offset, available, required }, valid_len: offset });
            }
            Err(error) => return Err(error),
        }
    }
    Ok(WalScan { header, frames, tail: WalTail::Clean, valid_len: offset })
}

/// Required record length reported for an incomplete record header, when known.
fn torn_required_len(bytes: &[u8], offset: usize) -> Option<usize> {
    let available = bytes.len().saturating_sub(offset);
    if available < WAL_RECORD_HEADER_BYTES {
        return None;
    }
    let frame_len = u32::from_le_bytes(bytes[offset + 9..offset + 13].try_into().ok()?) as usize;
    Some(frame_len.saturating_add(WAL_RECORD_LENGTH_AND_TRAILER_BYTES))
}

/// Parse a complete WAL file, rejecting a torn tail as corruption.
pub fn parse_wal(bytes: &[u8]) -> Result<WalFile> {
    let scan = scan_wal(bytes)?;
    if let WalTail::Torn { offset, available, required } = scan.tail {
        return Err(WalRecordError::RecordTruncated {
            offset,
            available,
            required: required.unwrap_or(WAL_RECORD_HEADER_BYTES),
        }
        .into());
    }
    Ok(WalFile { header: scan.header, frames: scan.frames })
}

/// Parse WAL bytes for recovery, dropping an incomplete trailing frame.
///
/// Checksum, length and LSN failures are still returned. Only a torn tail is
/// truncated from the parsed view.
pub fn parse_wal_recover(bytes: &[u8]) -> Result<WalFile> {
    let scan = scan_wal(bytes)?;
    Ok(WalFile { header: scan.header, frames: scan.frames })
}

/// Highest `frame_lsn` among committed transactions in `wal`.
pub fn committed_tail_lsn(wal: &WalFile) -> u64 {
    wal.frames.iter().filter(|frame| frame.frame_type == 0x04).map(|frame| frame.frame_lsn).max().unwrap_or(0)
}

/// Byte offsets of each frame start after the WAL header.
pub fn wal_frame_offsets(bytes: &[u8]) -> Result<Vec<usize>> {
    let scan = scan_wal(bytes)?;
    if matches!(scan.tail, WalTail::Torn { .. }) {
        return Err(Error::Corrupt("wal record truncated"));
    }
    let mut offset = WAL_HEADER_BYTES;
    let mut offsets = Vec::with_capacity(scan.frames.len());
    for frame in &scan.frames {
        offsets.push(offset);
        offset += WAL_RECORD_HEADER_BYTES + frame.body.len() + WAL_RECORD_TRAILER_BYTES;
    }
    Ok(offsets)
}

/// Return committed transaction count (`TxnCommit` frames with valid prefixes).
pub fn committed_transactions(wal: &WalFile) -> usize {
    wal.frames.iter().filter(|frame| frame.frame_type == 0x04).count()
}
