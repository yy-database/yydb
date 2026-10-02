//! Journal modes and `-wal` / `-shm` sidecar layout.
//!
//! Product surface remains one primary `.yydb` file. In [`JournalMode::Wal`],
//! durable writes append to `{path}-wal` and coordination metadata lives in
//! `{path}-shm`. Call [`crate::Connection::checkpoint`] to fold the WAL back
//! into the main file.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use yydb_types::{Error, Result};

/// How durable writes are published to disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JournalMode {
    /// Rewrite the main database file on each commit (default).
    #[default]
    Delete,
    /// Append commits to `{db}-wal` and keep `{db}-shm` index metadata.
    Wal,
}

impl JournalMode {
    /// Parse a journal_mode name (`delete` / `wal`).
    pub fn parse(name: &str) -> Result<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "delete" => Ok(Self::Delete),
            "wal" => Ok(Self::Wal),
            _ => Err(Error::Unsupported("journal_mode (use delete|wal)")),
        }
    }

    /// Stable lowercase name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Delete => "delete",
            Self::Wal => "wal",
        }
    }
}

/// Flags for [`crate::Connection::open_with_flags`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OpenFlags {
    /// Initial journal mode for this connection.
    pub journal_mode: JournalMode,
}

impl OpenFlags {
    /// Default flags (`delete` journal).
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable WAL + SHM sidecars.
    pub fn wal() -> Self {
        Self {
            journal_mode: JournalMode::Wal,
        }
    }
}

const WAL_MAGIC_V1: &[u8] = b"YYWL\x01";
const WAL_MAGIC_V2: &[u8] = b"YYWL\x02";
const SHM_MAGIC: &[u8] = b"YYSH\x01";
const SHM_BYTES: usize = 32;

/// `{path}-wal` companion path.
pub fn wal_path(db: &Path) -> PathBuf {
    sidecar(db, "-wal")
}

/// `{path}-shm` companion path.
pub fn shm_path(db: &Path) -> PathBuf {
    sidecar(db, "-shm")
}

fn sidecar(db: &Path, suffix: &str) -> PathBuf {
    let mut os = db.as_os_str().to_owned();
    os.push(suffix);
    PathBuf::from(os)
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ShmHeader {
    pub(crate) n_frames: u32,
    pub(crate) wal_bytes: u64,
}

pub(crate) fn ensure_wal_sidecars(db: &Path) -> Result<()> {
    let wal = wal_path(db);
    let shm = shm_path(db);
    if !wal.exists() {
        let mut file = File::create(&wal)?;
        file.write_all(WAL_MAGIC_V2)?;
        file.sync_all()?;
    }
    if !shm.exists() {
        write_shm(&shm, ShmHeader::default())?;
    }
    Ok(())
}

pub(crate) fn remove_wal_sidecars(db: &Path) -> Result<()> {
    let wal = wal_path(db);
    let shm = shm_path(db);
    if wal.exists() {
        fs::remove_file(&wal)?;
    }
    if shm.exists() {
        fs::remove_file(&shm)?;
    }
    Ok(())
}

pub(crate) fn read_shm(path: &Path) -> Result<ShmHeader> {
    if !path.exists() {
        return Ok(ShmHeader::default());
    }
    let bytes = fs::read(path)?;
    if bytes.len() < SHM_BYTES {
        return Err(Error::Corrupt("shm too short"));
    }
    if &bytes[..SHM_MAGIC.len()] != SHM_MAGIC {
        return Err(Error::Corrupt("unknown shm header"));
    }
    let n_frames = u32::from_le_bytes(bytes[5..9].try_into().unwrap());
    let wal_bytes = u64::from_le_bytes(bytes[9..17].try_into().unwrap());
    Ok(ShmHeader {
        n_frames,
        wal_bytes,
    })
}

pub(crate) fn write_shm(path: &Path, header: ShmHeader) -> Result<()> {
    let mut buf = vec![0_u8; SHM_BYTES];
    buf[..SHM_MAGIC.len()].copy_from_slice(SHM_MAGIC);
    buf[5..9].copy_from_slice(&header.n_frames.to_le_bytes());
    buf[9..17].copy_from_slice(&header.wal_bytes.to_le_bytes());
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)?;
    file.write_all(&buf)?;
    file.sync_all()?;
    Ok(())
}

/// Append one full-state snapshot frame to the WAL and refresh SHM.
pub(crate) fn append_snapshot_frame(db: &Path, payload: &[u8]) -> Result<()> {
    ensure_wal_sidecars(db)?;
    let wal = wal_path(db);
    let bytes = fs::read(&wal)?;
    let scanned = scan_wal(&bytes)?;
    let len = u32::try_from(payload.len()).map_err(|_| Error::Corrupt("wal frame too large"))?;
    let n_frames = scanned
        .n_frames
        .checked_add(1)
        .ok_or(Error::Corrupt("wal frame count exhausted"))?;
    let mut frame = Vec::with_capacity(5 + payload.len());
    frame.push(1);
    frame.extend(len.to_le_bytes());
    frame.extend(payload);
    let mut file = OpenOptions::new().write(true).open(&wal)?;
    file.set_len(scanned.valid_len as u64)?;
    file.seek(SeekFrom::Start(scanned.valid_len as u64))?;
    file.write_all(&frame)?;
    if scanned.version == 2 {
        file.write_all(blake3::hash(&frame).as_bytes())?;
    }
    file.flush()?;
    file.sync_all()?;

    let meta = file.metadata()?;
    write_shm(
        &shm_path(db),
        ShmHeader {
            n_frames,
            wal_bytes: meta.len(),
        },
    )?;
    Ok(())
}

/// Replay all snapshot frames after `base` (later frames win).
pub(crate) fn replay_wal_snapshots(db: &Path, base: Vec<u8>) -> Result<Vec<u8>> {
    let wal = wal_path(db);
    if !wal.exists() {
        return Ok(base);
    }
    let bytes = fs::read(&wal)?;
    let scanned = scan_wal(&bytes)?;
    Ok(scanned
        .last_payload
        .map_or(base, |range| bytes[range].to_vec()))
}

struct WalScan {
    version: u8,
    valid_len: usize,
    n_frames: u32,
    last_payload: Option<std::ops::Range<usize>>,
}

fn scan_wal(bytes: &[u8]) -> Result<WalScan> {
    if bytes.len() < WAL_MAGIC_V1.len() {
        return Err(Error::Corrupt("unknown wal header"));
    }
    let header = &bytes[..WAL_MAGIC_V1.len()];
    let version = if header == WAL_MAGIC_V1 {
        1
    } else if header == WAL_MAGIC_V2 {
        2
    } else {
        return Err(Error::Corrupt("unknown wal header"));
    };
    let mut cursor = WAL_MAGIC_V1.len();
    let mut scanned = WalScan {
        version,
        valid_len: cursor,
        n_frames: 0,
        last_payload: None,
    };
    while cursor < bytes.len() {
        let frame_start = cursor;
        let kind = match bytes.get(cursor) {
            Some(kind) => *kind,
            None => break,
        };
        cursor += 1;
        if kind != 1 {
            return Err(Error::Corrupt("unsupported wal frame type"));
        }
        if cursor + 4 > bytes.len() {
            break;
        }
        let len = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap()) as usize;
        cursor += 4;
        let payload_end = cursor
            .checked_add(len)
            .ok_or(Error::Corrupt("wal length overflow"))?;
        if payload_end > bytes.len() {
            break;
        }
        let payload_start = cursor;
        if version == 2 {
            if payload_end + 32 > bytes.len() {
                break;
            }
            let expected = blake3::hash(&bytes[frame_start..payload_end]);
            if bytes[payload_end..payload_end + 32] != *expected.as_bytes() {
                return Err(Error::Corrupt("wal frame checksum mismatch"));
            }
            cursor = payload_end + 32;
        } else {
            cursor = payload_end;
        }
        scanned.valid_len = cursor;
        scanned.n_frames = scanned
            .n_frames
            .checked_add(1)
            .ok_or(Error::Corrupt("wal frame count exhausted"))?;
        scanned.last_payload = Some(payload_start..payload_end);
    }
    Ok(scanned)
}

pub(crate) fn truncate_wal(db: &Path) -> Result<()> {
    let wal = wal_path(db);
    let mut file = File::create(&wal)?;
    file.write_all(WAL_MAGIC_V2)?;
    file.sync_all()?;
    write_shm(&shm_path(db), ShmHeader::default())?;
    Ok(())
}

pub(crate) fn wal_frame_count(db: &Path) -> Result<u32> {
    let wal = wal_path(db);
    if !wal.exists() {
        return Ok(0);
    }
    Ok(scan_wal(&fs::read(wal)?)?.n_frames)
}

/// Detect existing sidecars (useful for `info`).
pub fn sidecar_status(db: &Path) -> Result<(bool, bool, u32)> {
    let wal = wal_path(db);
    let shm = shm_path(db);
    let frames = wal_frame_count(db)?;
    Ok((wal.exists(), shm.exists(), frames))
}

#[allow(dead_code)]
pub(crate) fn read_exact_prefix(path: &Path, n: usize) -> Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let mut buf = vec![0_u8; n];
    file.read_exact(&mut buf)?;
    Ok(buf)
}
