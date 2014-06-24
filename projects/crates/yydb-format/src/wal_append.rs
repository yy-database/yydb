//! WAL v0 frame encoding and append helper.

use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use yydb_types::{Error, Result};

use crate::blob_refs::{commit_digest, encode_blob_refs_body, BlobManifestDelta, FRAME_BLOB_REFS};
use crate::crc32c::crc32c;
use crate::header::PAGE_SIZE;
use crate::wal::{
    parse_frame, parse_header, parse_wal, wal_frame_offsets, WalFile, WAL_MAGIC,
};

const FRAME_TXN_BEGIN: u8 = 0x01;
const FRAME_PAGE_IMAGE: u8 = 0x02;
const FRAME_TXN_COMMIT: u8 = 0x04;

/// Append-only WAL writer for page-image transactions.
pub struct WalWriter {
    path: PathBuf,
    #[allow(dead_code)]
    database_id: [u8; 16],
    next_lsn: u64,
    next_txid: u64,
}

impl WalWriter {
    /// Open or create a WAL sidecar for `database_id`.
    pub fn open(path: impl AsRef<Path>, database_id: [u8; 16]) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut next_lsn = 1_u64;
        let mut next_txid = 1_u64;
        if path.exists() {
            let bytes = std::fs::read(&path)?;
            let wal = parse_wal(&bytes)?;
            next_lsn = wal
                .frames
                .iter()
                .map(|frame| frame.frame_lsn)
                .max()
                .unwrap_or(0)
                + 1;
            next_txid = wal
                .frames
                .iter()
                .filter(|frame| frame.frame_type == FRAME_TXN_BEGIN)
                .count() as u64
                + 1;
        } else {
            let header = encode_wal_header(database_id, 0);
            let mut file = File::create(&path)?;
            file.write_all(&header)?;
            file.sync_all()?;
        }
        Ok(Self {
            path,
            database_id,
            next_lsn,
            next_txid,
        })
    }

    /// Append one committed transaction that publishes `pages` and optional blob refs.
    pub fn append_commit(
        &mut self,
        pages: &[(u32, Vec<u8>)],
        blob_refs: &[BlobManifestDelta],
    ) -> Result<u64> {
        let txid = self.next_txid;
        self.next_txid += 1;
        let mut file = OpenOptions::new()
            .append(true)
            .write(true)
            .open(&self.path)?;
        file.write_all(&encode_frame(
            FRAME_TXN_BEGIN,
            self.next_lsn,
            &txid.to_le_bytes(),
        )?)?;
        self.next_lsn += 1;
        for (page_id, image) in pages {
            if image.len() != PAGE_SIZE {
                return Err(Error::Corrupt("wal page image wrong size"));
            }
            let mut body = Vec::with_capacity(20 + PAGE_SIZE);
            body.extend_from_slice(&txid.to_le_bytes());
            body.extend_from_slice(&page_id.to_le_bytes());
            body.extend_from_slice(&1_u64.to_le_bytes());
            body.extend_from_slice(image);
            file.write_all(&encode_frame(FRAME_PAGE_IMAGE, self.next_lsn, &body)?)?;
            self.next_lsn += 1;
        }
        if !blob_refs.is_empty() {
            let body = encode_blob_refs_body(txid, blob_refs)?;
            file.write_all(&encode_frame(FRAME_BLOB_REFS, self.next_lsn, &body)?)?;
            self.next_lsn += 1;
        }
        let commit_lsn = self.next_lsn;
        let digest = commit_digest(pages, blob_refs);
        let mut commit_body = Vec::with_capacity(56);
        commit_body.extend_from_slice(&txid.to_le_bytes());
        commit_body.extend_from_slice(&commit_lsn.to_le_bytes());
        commit_body.extend_from_slice(&(pages.len() as u32).to_le_bytes());
        commit_body.extend_from_slice(&(blob_refs.len() as u32).to_le_bytes());
        commit_body.extend_from_slice(&digest);
        file.write_all(&encode_frame(FRAME_TXN_COMMIT, commit_lsn, &commit_body)?)?;
        self.next_lsn += 1;
        file.sync_all()?;
        Ok(commit_lsn)
    }

    /// Sidecar path.
    pub fn path(&self) -> &Path {
        &self.path
    }
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
    let mut in_txn = false;
    let mut pending: Vec<(u32, Vec<u8>)> = Vec::new();
    for frame in &wal.frames {
        match frame.frame_type {
            FRAME_TXN_BEGIN => {
                in_txn = true;
                pending.clear();
            }
            FRAME_PAGE_IMAGE if in_txn => {
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
    let last_start = offsets
        .last()
        .copied()
        .ok_or_else(|| Error::Corrupt("wal missing trailing commit frame"))?;
    Ok(bytes[0..last_start].to_vec())
}

/// Byte length of the recoverable WAL prefix (header plus complete frames).
pub fn wal_recoverable_byte_len(bytes: &[u8]) -> Result<usize> {
    let (_, mut offset) = parse_header(bytes)?;
    while offset < bytes.len() {
        match parse_frame(bytes, offset) {
            Ok((_, consumed)) => offset += consumed,
            Err(_) => break,
        }
    }
    Ok(offset)
}

/// Truncate WAL bytes to the recoverable prefix, dropping a torn tail frame.
pub fn truncate_wal_to_recoverable_prefix(bytes: &[u8]) -> Result<Vec<u8>> {
    let end = wal_recoverable_byte_len(bytes)?;
    Ok(bytes[0..end].to_vec())
}

/// Truncate WAL bytes immediately after `frame_index` (inclusive).
pub fn truncate_wal_after_frame(bytes: &[u8], frame_index: usize) -> Result<Vec<u8>> {
    let offsets = wal_frame_offsets(bytes)?;
    let start = offsets
        .get(frame_index)
        .copied()
        .ok_or_else(|| Error::Corrupt("wal frame index out of range"))?;
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
        .ok_or_else(|| Error::Corrupt("wal missing txn begin"))?;
    let mut last_page_index = None;
    for (index, frame) in wal.frames.iter().enumerate().skip(begin_index) {
        match frame.frame_type {
            FRAME_PAGE_IMAGE => last_page_index = Some(index),
            FRAME_TXN_COMMIT => break,
            _ => {}
        }
    }
    let page_index = last_page_index.ok_or_else(|| Error::Corrupt("wal txn missing page image"))?;
    truncate_wal_after_frame(bytes, page_index)
}
