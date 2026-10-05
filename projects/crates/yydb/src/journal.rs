//! Journal modes and `{path}-wal` sidecar paths for `YDPG` / `YYWL` v0.

use std::{
    fs,
    path::{Path, PathBuf},
};

use yydb_format::{parse_wal, WAL_MAGIC};
use yydb_types::{Error, Result};

/// How durable writes are published to disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JournalMode {
    /// Rewrite the main database file on each commit (default).
    #[default]
    Delete,
    /// Append page-image transactions to `{db}-wal`.
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

    /// Enable `YYWL` v0 journaling.
    pub fn wal() -> Self {
        Self {
            journal_mode: JournalMode::Wal,
        }
    }
}

/// `{path}-wal` companion path.
pub fn wal_path(db: &Path) -> PathBuf {
    sidecar(db, "-wal")
}

/// Reserved `{path}-shm` path for future coordination metadata.
pub fn shm_path(db: &Path) -> PathBuf {
    sidecar(db, "-shm")
}

fn sidecar(db: &Path, suffix: &str) -> PathBuf {
    let mut os = db.as_os_str().to_owned();
    os.push(suffix);
    PathBuf::from(os)
}

/// Detect WAL sidecar presence and frame count for `YYWL` v0 files.
pub fn sidecar_status(db: &Path) -> Result<(bool, bool, u32)> {
    let wal = wal_path(db);
    let shm = shm_path(db);
    let frames = if wal.exists() {
        let bytes = fs::read(&wal)?;
        if bytes.get(0..5) == Some(WAL_MAGIC.as_slice()) {
            parse_wal(&bytes)?.frames.len() as u32
        } else {
            return Err(Error::Corrupt("wal magic mismatch"));
        }
    } else {
        0
    };
    Ok((wal.exists(), shm.exists(), frames))
}
