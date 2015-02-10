//! WAL v0 frame encoding and append helper.

use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use yydb_types::{Error, Result};

use crate::{
    blob_refs::{BlobManifestDelta, FRAME_BLOB_REFS, commit_digest, encode_blob_refs_body},
    crc32c::crc32c,
    header::PAGE_SIZE,
    wal::{WAL_HEADER_BYTES, WAL_MAGIC, WalFile, WalTail, parse_frame, parse_wal, scan_wal, wal_frame_offsets},
};

const FRAME_TXN_BEGIN: u8 = 0x01;
const FRAME_PAGE_IMAGE: u8 = 0x02;
const FRAME_TXN_COMMIT: u8 = 0x04;
const WRITER_POISONED: &str = "wal writer poisoned after a failed append";

/// Durability level declared by a [`WalWriter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WalDurability {
    /// Append every frame of one transaction, then call [`WalSink::sync`] once
    /// before reporting the commit LSN. A failed sync is not a commit.
    #[default]
    SyncOnCommit,
}

/// Result of one startup WAL recovery pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    /// Torn tail bytes dropped by this pass.
    pub dropped_torn_bytes: u64,
    /// Complete frames after truncation.
    pub frames: usize,
    /// Frames above the database checkpoint LSN.
    pub frames_after_checkpoint: usize,
    /// Committed transactions found in the WAL.
    pub committed_transactions: usize,
    /// Highest commit LSN in the WAL.
    pub committed_lsn: u64,
}

/// Append-only byte sink behind [`WalWriter`].
///
/// Production uses [`FileWalSink`]. Tests inject failing sinks to model disk
/// full, short writes and lost syncs without touching the file format.
pub trait WalSink: Send {
    /// Append the whole buffer or fail. A short write must be reported as an error.
    fn append(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// Durability barrier for every byte appended so far.
    fn sync(&mut self) -> io::Result<()>;
}

/// File-backed [`WalSink`] using `fsync` (`sync_all`) for both data and length.
pub struct FileWalSink {
    file: File,
}

impl FileWalSink {
    /// Open `path` for appending, creating it when absent.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self { file })
    }
}

impl WalSink for FileWalSink {
    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.file.write_all(bytes)
    }

    fn sync(&mut self) -> io::Result<()> {
        self.file.sync_all()
    }
}

/// Append-only WAL writer for page-image transactions.
pub struct WalWriter {
    path: PathBuf,
    database_id: [u8; 16],
    next_lsn: u64,
    next_txid: u64,
    durability: WalDurability,
    sink: Box<dyn WalSink>,
    poisoned: bool,
}

impl WalWriter {
    /// Open or create a WAL sidecar for `database_id`.
    ///
    /// `base_checkpoint_lsn` is the database checkpoint LSN the sidecar continues
    /// from. New records always start above it, so a WAL created after a
    /// checkpoint cannot reuse LSNs that recovery already treats as folded.
    ///
    /// A torn tail is rejected here. Callers must truncate the tail through
    /// recovery before appending, otherwise new records would follow garbage.
    pub fn open(path: impl AsRef<Path>, database_id: [u8; 16], base_checkpoint_lsn: u64) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut next_lsn = base_checkpoint_lsn + 1;
        let mut next_txid = 1_u64;
        if path.exists() {
            let bytes = std::fs::read(&path)?;
            let scan = scan_wal(&bytes)?;
            if !matches!(scan.tail, WalTail::Clean) {
                return Err(Error::Corrupt("wal torn tail requires recovery before append"));
            }
            let tail_lsn = scan.frames.iter().map(|frame| frame.frame_lsn).max().unwrap_or(0);
            next_lsn = tail_lsn.max(base_checkpoint_lsn) + 1;
            next_txid = scan.frames.iter().filter(|frame| frame.frame_type == FRAME_TXN_BEGIN).count() as u64 + 1;
        }
        else {
            let header = encode_wal_header(database_id, base_checkpoint_lsn);
            let mut file = File::create(&path)?;
            file.write_all(&header)?;
            file.sync_all()?;
        }
        let sink = FileWalSink::open(&path)?;
        Ok(Self {
            path,
            database_id,
            next_lsn,
            next_txid,
            durability: WalDurability::SyncOnCommit,
            sink: Box::new(sink),
            poisoned: false,
        })
    }

    /// Build a writer over a caller-supplied sink.
    ///
    /// The path is metadata only. Tests use this to inject disk-full and short
    /// write failures while keeping the v0 record format unchanged.
    pub fn with_sink(path: impl AsRef<Path>, database_id: [u8; 16], sink: Box<dyn WalSink>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            database_id,
            next_lsn: 1,
            next_txid: 1,
            durability: WalDurability::SyncOnCommit,
            sink,
            poisoned: false,
        }
    }

    /// Declared durability level.
    pub fn durability(&self) -> WalDurability {
        self.durability
    }

    /// Append one committed transaction that publishes `pages` and optional blob refs.
    ///
    /// The returned LSN is reported only after the sink accepted every frame and
    /// completed the durability barrier. Any append or sync failure poisons this
    /// writer: further appends fail until the WAL is recovered and reopened.
    pub fn append_commit(&mut self, pages: &[(u32, Vec<u8>)], blob_refs: &[BlobManifestDelta]) -> Result<u64> {
        if self.poisoned {
            return Err(Error::Corrupt(WRITER_POISONED));
        }
        let txid = self.next_txid;
        let (bytes, commit_lsn, next_lsn) = encode_commit(self.next_lsn, txid, pages, blob_refs)?;
        let written = self.sink.append(&bytes).and_then(|()| match self.durability {
            WalDurability::SyncOnCommit => self.sink.sync(),
        });
        match written {
            Ok(()) => {
                self.next_txid = txid + 1;
                self.next_lsn = next_lsn;
                Ok(commit_lsn)
            }
            Err(error) => {
                self.poisoned = true;
                Err(Error::Io(error))
            }
        }
    }

    /// Sidecar path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Database id printed into a fresh WAL header.
    pub fn database_id(&self) -> [u8; 16] {
        self.database_id
    }
}

/// Encode one complete transaction and report the commit LSN plus the next LSN.
fn encode_commit(
    start_lsn: u64,
    txid: u64,
    pages: &[(u32, Vec<u8>)],
    blob_refs: &[BlobManifestDelta],
) -> Result<(Vec<u8>, u64, u64)> {
    let mut lsn = start_lsn;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&encode_frame(FRAME_TXN_BEGIN, lsn, &txid.to_le_bytes())?);
    lsn += 1;
    for (page_id, image) in pages {
        if image.len() != PAGE_SIZE {
            return Err(Error::Corrupt("wal page image wrong size"));
        }
        let mut body = Vec::with_capacity(20 + PAGE_SIZE);
        body.extend_from_slice(&txid.to_le_bytes());
        body.extend_from_slice(&page_id.to_le_bytes());
        body.extend_from_slice(&1_u64.to_le_bytes());
        body.extend_from_slice(image);
        bytes.extend_from_slice(&encode_frame(FRAME_PAGE_IMAGE, lsn, &body)?);
        lsn += 1;
    }
    if !blob_refs.is_empty() {
        let body = encode_blob_refs_body(txid, blob_refs)?;
        bytes.extend_from_slice(&encode_frame(FRAME_BLOB_REFS, lsn, &body)?);
        lsn += 1;
    }
    let commit_lsn = lsn;
    let digest = commit_digest(pages, blob_refs);
    let mut commit_body = Vec::with_capacity(56);
    commit_body.extend_from_slice(&txid.to_le_bytes());
    commit_body.extend_from_slice(&commit_lsn.to_le_bytes());
    commit_body.extend_from_slice(&(pages.len() as u32).to_le_bytes());
    commit_body.extend_from_slice(&(blob_refs.len() as u32).to_le_bytes());
    commit_body.extend_from_slice(&digest);
    bytes.extend_from_slice(&encode_frame(FRAME_TXN_COMMIT, commit_lsn, &commit_body)?);
    lsn += 1;
    Ok((bytes, commit_lsn, lsn))
}

/// Drop every record while keeping the existing file header intact.
///
/// The header checksum and format version stay valid, so a crash after this
/// truncation leaves an openable WAL with no replayable records.
pub fn truncate_wal_records(path: &Path) -> Result<()> {
    let file = OpenOptions::new().write(true).open(path)?;
    file.set_len(WAL_HEADER_BYTES as u64)?;
    file.sync_all()?;
    Ok(())
}

/// Truncate a torn tail from a WAL file and fsync the result.
///
/// Returns the number of dropped bytes. Damaged records (checksum, length, LSN)
/// are returned as errors and the file is left unchanged.
pub fn truncate_wal_torn_tail(path: &Path) -> Result<usize> {
    let bytes = std::fs::read(path)?;
    let scan = scan_wal(&bytes)?;
    if matches!(scan.tail, WalTail::Clean) {
        return Ok(0);
    }
    let dropped = bytes.len() - scan.valid_len;
    let file = OpenOptions::new().write(true).open(path)?;
    file.set_len(scan.valid_len as u64)?;
    file.sync_all()?;
    Ok(dropped)
}

/// Encode a v0 WAL file header with checksum.
pub fn encode_wal_header(database_id: [u8; 16], base_checkpoint_lsn: u64) -> Vec<u8> {
    let mut header = Vec::with_capacity(37);
    header.extend_from_slice(WAL_MAGIC);
    header.extend_from_slice(&database_id);
    header.extend_from_slice(&base_checkpoint_lsn.to_le_bytes());
    header.extend_from_slice(&0_u32.to_le_bytes());
    header.extend_from_slice(&crc32c(&header).to_le_bytes());
    header
}

/// Encode one WAL frame with checksum trailer.
pub fn encode_frame(frame_type: u8, frame_lsn: u64, body: &[u8]) -> Result<Vec<u8>> {
    let frame_len = 1 + 8 + body.len();
    if frame_len < 9 {
        return Err(Error::Corrupt("wal frame too small"));
    }
    let mut frame = Vec::with_capacity(13 + body.len() + 4);
    frame.push(frame_type);
    frame.extend_from_slice(&frame_lsn.to_le_bytes());
    frame.extend_from_slice(&(frame_len as u32).to_le_bytes());
    frame.extend_from_slice(body);
    frame.extend_from_slice(&crc32c(&frame).to_le_bytes());
    Ok(frame)
}

/// Apply committed WAL page images onto `pages`, last committed txn wins per page.
pub fn replay_wal_pages(wal: &WalFile, pages: &mut BTreeMap<u32, Vec<u8>>) -> Result<()> {
    replay_wal_pages_after(wal, 0, pages)
}

/// Apply committed WAL page images with `frame_lsn > checkpoint_lsn`.
///
/// Every frame at or below the database checkpoint LSN is already in the main
/// file and must not be replayed after a checkpoint that crashed before its WAL
/// truncation.
pub fn replay_wal_pages_after(wal: &WalFile, checkpoint_lsn: u64, pages: &mut BTreeMap<u32, Vec<u8>>) -> Result<()> {
    let mut in_txn = false;
    let mut pending: Vec<(u32, Vec<u8>)> = Vec::new();
    for frame in &wal.frames {
        if frame.frame_lsn <= checkpoint_lsn {
            if frame.frame_type == FRAME_TXN_COMMIT {
                in_txn = false;
                pending.clear();
            }
            continue;
        }
        match frame.frame_type {
            FRAME_TXN_BEGIN => {
                in_txn = true;
                pending.clear();
            }
            FRAME_PAGE_IMAGE if in_txn => {
                if frame.body.len() < 20 {
                    return Err(Error::Corrupt("wal replay page body truncated"));
                }
                let page_id = u32::from_le_bytes(frame.body[8..12].try_into().unwrap());
                let image = frame.body[20..].to_vec();
                if image.len() != PAGE_SIZE {
                    return Err(Error::Corrupt("wal replay page wrong size"));
                }
                pending.push((page_id, image));
            }
            FRAME_BLOB_REFS if in_txn => {
                // Catalog pages carry committed manifest state; blob refs are validated at commit.
            }
            FRAME_TXN_COMMIT => {
                for (page_id, image) in pending.drain(..) {
                    pages.insert(page_id, image);
                }
                in_txn = false;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Read a WAL sidecar from disk, tolerating a truncated tail frame.
pub fn read_wal_file(path: &Path) -> Result<WalFile> {
    crate::wal::parse_wal_recover(&std::fs::read(path)?)
}

/// Resolve `<main>-wal` for a database main file path.
pub fn wal_sidecar_path(main: &Path) -> PathBuf {
    let mut sidecar = main.as_os_str().to_owned();
    sidecar.push("-wal");
    PathBuf::from(sidecar)
}

/// Drop the trailing `TxnCommit` frame when present.
///
/// Used by crash-injection tests to simulate durability loss between the final
/// `PageImage` and `TxnCommit`.
pub fn strip_trailing_txn_commit(bytes: &[u8]) -> Result<Vec<u8>> {
    let wal = parse_wal(bytes)?;
    if wal.frames.last().map(|frame| frame.frame_type) != Some(FRAME_TXN_COMMIT) {
        return Ok(bytes.to_vec());
    }
    let offsets = wal_frame_offsets(bytes)?;
    let last_start = offsets.last().copied().ok_or(Error::Corrupt("wal missing trailing commit frame"))?;
    Ok(bytes[0..last_start].to_vec())
}

/// Byte length of the recoverable WAL prefix (header plus complete frames).
///
/// Only a torn tail is reduced. Checksum, length and LSN failures keep the whole
/// input invalid and are returned as errors.
pub fn wal_recoverable_byte_len(bytes: &[u8]) -> Result<usize> {
    let scan = scan_wal(bytes)?;
    Ok(match scan.tail {
        WalTail::Clean => bytes.len(),
        WalTail::Torn { .. } => scan.valid_len,
    })
}

/// Truncate WAL bytes to the recoverable prefix, dropping a torn tail frame.
pub fn truncate_wal_to_recoverable_prefix(bytes: &[u8]) -> Result<Vec<u8>> {
    let end = wal_recoverable_byte_len(bytes)?;
    Ok(bytes[0..end].to_vec())
}

/// Truncate WAL bytes immediately after `frame_index` (inclusive).
pub fn truncate_wal_after_frame(bytes: &[u8], frame_index: usize) -> Result<Vec<u8>> {
    let offsets = wal_frame_offsets(bytes)?;
    let start = offsets.get(frame_index).copied().ok_or(Error::Corrupt("wal frame index out of range"))?;
    let (_, consumed) = parse_frame(bytes, start)?;
    Ok(bytes[0..start + consumed].to_vec())
}

/// Drop frames after the last `PageImage` in the final open transaction.
///
/// Simulates a crash after catalog page images were appended but before
/// `.yydx` `BlobRefs` or `TxnCommit`.
pub fn strip_incomplete_txn_after_last_page_image(bytes: &[u8]) -> Result<Vec<u8>> {
    let wal = parse_wal(bytes)?;
    let begin_index = wal
        .frames
        .iter()
        .rposition(|frame| frame.frame_type == FRAME_TXN_BEGIN)
        .ok_or(Error::Corrupt("wal missing txn begin"))?;
    let mut last_page_index = None;
    for (index, frame) in wal.frames.iter().enumerate().skip(begin_index) {
        match frame.frame_type {
            FRAME_PAGE_IMAGE => last_page_index = Some(index),
            FRAME_TXN_COMMIT => break,
            _ => {}
        }
    }
    let page_index = last_page_index.ok_or(Error::Corrupt("wal txn missing page image"))?;
    truncate_wal_after_frame(bytes, page_index)
}
