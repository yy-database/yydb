//! Journal modes and `-wal` / `-shm` sidecar layout.
//!
//! Product surface remains one primary `.yydb` file. In [`JournalMode::Wal`],
//! durable writes append to `{path}-wal` and coordination metadata lives in
//! `{path}-shm`. Call [`crate::Connection::checkpoint`] to fold the WAL back
//! into the main file.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
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

const WAL_MAGIC: &[u8] = b"YYWL\x01";
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
        file.write_all(WAL_MAGIC)?;
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
    fs::write(path, buf).map_err(Error::Io)
}

/// Append one full-state snapshot frame to the WAL and refresh SHM.
pub(crate) fn append_snapshot_frame(db: &Path, payload: &[u8]) -> Result<()> {
    ensure_wal_sidecars(db)?;
    let wal = wal_path(db);
    let mut file = OpenOptions::new().append(true).open(&wal)?;
    // type 1 = full snapshot
    file.write_all(&[1])?;
    let len = u32::try_from(payload.len()).map_err(|_| Error::Corrupt("wal frame too large"))?;
    file.write_all(&len.to_le_bytes())?;
    file.write_all(payload)?;
    file.flush()?;

    let meta = file.metadata()?;
    let mut shm = read_shm(&shm_path(db))?;
    shm.n_frames = shm.n_frames.saturating_add(1);
    shm.wal_bytes = meta.len();
    write_shm(&shm_path(db), shm)?;
    Ok(())
}

/// Replay all snapshot frames after `base` (later frames win).
pub(crate) fn replay_wal_snapshots(db: &Path, mut base: Vec<u8>) -> Result<Vec<u8>> {
    let wal = wal_path(db);
    if !wal.exists() {
        return Ok(base);
    }
    let bytes = fs::read(&wal)?;
    if bytes.is_empty() {
        return Ok(base);
    }
    if bytes.len() < WAL_MAGIC.len() || &bytes[..WAL_MAGIC.len()] != WAL_MAGIC {
        return Err(Error::Corrupt("unknown wal header"));
    }
    let mut cursor = WAL_MAGIC.len();
    while cursor < bytes.len() {
        let kind = *bytes
            .get(cursor)
            .ok_or(Error::Corrupt("truncated wal frame"))?;
        cursor += 1;
        if kind != 1 {
            return Err(Error::Corrupt("unsupported wal frame type"));
        }
        if cursor + 4 > bytes.len() {
            return Err(Error::Corrupt("truncated wal length"));
        }
        let len = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap()) as usize;
        cursor += 4;
        if cursor + len > bytes.len() {
            return Err(Error::Corrupt("truncated wal payload"));
        }
        base = bytes[cursor..cursor + len].to_vec();
        cursor += len;
    }
    Ok(base)
}

pub(crate) fn truncate_wal(db: &Path) -> Result<()> {
    let wal = wal_path(db);
    let mut file = File::create(&wal)?;
    file.write_all(WAL_MAGIC)?;
    write_shm(&shm_path(db), ShmHeader::default())?;
    Ok(())
}

pub(crate) fn wal_frame_count(db: &Path) -> Result<u32> {
    Ok(read_shm(&shm_path(db))?.n_frames)
}

/// Detect existing sidecars (useful for `info`).
pub fn sidecar_status(db: &Path) -> Result<(bool, bool, u32)> {
    let wal = wal_path(db);
    let shm = shm_path(db);
    let frames = if shm.exists() {
        read_shm(&shm)?.n_frames
    } else {
        0
    };
    Ok((wal.exists(), shm.exists(), frames))
}

#[allow(dead_code)]
pub(crate) fn read_exact_prefix(path: &Path, n: usize) -> Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let mut buf = vec![0_u8; n];
    file.read_exact(&mut buf)?;
    Ok(buf)
}
