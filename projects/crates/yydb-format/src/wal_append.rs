//! WAL v0 frame encoding and append helper.

use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use yydb_types::{Error, Result};

use crate::crc32c::crc32c;
use crate::header::PAGE_SIZE;
use crate::wal::{parse_wal, wal_frame_offsets, WalFile, WAL_MAGIC};

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

    /// Append one committed transaction that publishes `pages`.
    pub fn append_commit(&mut self, pages: &[(u32, Vec<u8>)]) -> Result<u64> {
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
        let commit_lsn = self.next_lsn;
        let mut commit_body = Vec::with_capacity(24);
        commit_body.extend_from_slice(&txid.to_le_bytes());
        commit_body.extend_from_slice(&commit_lsn.to_le_bytes());
        commit_body.extend_from_slice(&(pages.len() as u32).to_le_bytes());
        commit_body.extend_from_slice(&0_u32.to_le_bytes());
        commit_body.extend_from_slice(&[0_u8; 32]);
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

pub fn encode_wal_header(database_id: [u8; 16], base_checkpoint_lsn: u64) -> Vec<u8> {
    let mut header = Vec::with_capacity(37);
    header.extend_from_slice(WAL_MAGIC);
    header.extend_from_slice(&database_id);
    header.extend_from_slice(&base_checkpoint_lsn.to_le_bytes());
    header.extend_from_slice(&0_u32.to_le_bytes());
    header.extend_from_slice(&crc32c(&header).to_le_bytes());
    header
}

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

pub fn read_wal_file(path: &Path) -> Result<WalFile> {
    parse_wal(&std::fs::read(path)?)
}

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
