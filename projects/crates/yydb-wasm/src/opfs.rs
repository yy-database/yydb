//! OPFS host profile capability probe and persistent-open gate (Living `08`).
//!
//! Pure Rust surface for host-testable gates. Actual OPFS I/O is not implemented yet.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use yydb::{Error, Result};

fn writer_locks() -> &'static Mutex<HashSet<String>> {
    static LOCKS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Persistent storage format selected for an OPFS open attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistentStorageMode {
    /// Single-file `.yydb` layout (Living `07`).
    Yydb,
    /// Multi-file `.yydx` + blob directory layout (Living `07`).
    Yydx,
}

impl PersistentStorageMode {
    fn from_path(path: &str) -> Result<Self> {
        if path.ends_with(".yydx") {
            return Ok(Self::Yydx);
        }
        if path.ends_with(".yydb") {
            return Ok(Self::Yydb);
        }
        Err(Error::Unsupported(
            "persistent OPFS path must end with .yydb or .yydx",
        ))
    }
}

/// Structured OPFS host capability snapshot (Living `08` §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OpfsCapabilities {
    /// Can open or create `.yydb` / `.yydx` under an OPFS root.
    pub persistent_open: bool,
    /// At most one writer per logical database.
    pub single_writer: bool,
    /// Staging write, verify, then atomically publish visible root.
    pub atomic_publish: bool,
    /// Flush / sync boundary before durable success.
    pub durability_sync: bool,
    /// Cross-context writer fencing (`file_lock` or equivalent).
    pub file_lock: bool,
    /// Quota observation and structured exhaustion errors.
    pub quota_report: bool,
    /// Range reads for blob segments (required for `.yydx`).
    pub range_read: bool,
    /// Host exposes memory only (must not be set for persistent open).
    pub memory_only: bool,
}

impl OpfsCapabilities {
    /// All capabilities required for persistent `.yydb` on OPFS.
    pub fn full_yydb_profile() -> Self {
        Self {
            persistent_open: true,
            single_writer: true,
            atomic_publish: true,
            durability_sync: true,
            file_lock: true,
            quota_report: true,
            range_read: false,
            memory_only: false,
        }
    }

    /// All capabilities required for persistent `.yydx` on OPFS.
    pub fn full_yydx_profile() -> Self {
        Self {
            range_read: true,
            ..Self::full_yydb_profile()
        }
    }
}

/// Validate capabilities for the requested storage mode without opening I/O.
pub fn validate_opfs_capabilities(
    caps: &OpfsCapabilities,
    mode: PersistentStorageMode,
) -> Result<()> {
    if caps.memory_only {
        return Err(Error::Unsupported(
            "persistent OPFS open rejected: host profile is memory_only",
        ));
    }
    require(
        caps.persistent_open,
        "missing OPFS capability: persistent_open",
    )?;
    require(caps.single_writer, "missing OPFS capability: single_writer")?;
    require(
        caps.atomic_publish,
        "missing OPFS capability: atomic_publish",
    )?;
    require(
        caps.durability_sync,
        "missing OPFS capability: durability_sync",
    )?;
    require(caps.file_lock, "missing OPFS capability: file_lock")?;
    require(caps.quota_report, "missing OPFS capability: quota_report")?;
    if mode == PersistentStorageMode::Yydx {
        require(caps.range_read, "missing OPFS capability: range_read")?;
    }
    Ok(())
}

fn require(present: bool, message: &'static str) -> Result<()> {
    if present {
        Ok(())
    } else {
        Err(Error::Unsupported(message))
    }
}

/// Exclusive writer lease for one logical OPFS database path (Living `08` §4).
#[derive(Debug)]
pub struct OpfsWriterLease {
    path: String,
}

impl Drop for OpfsWriterLease {
    fn drop(&mut self) {
        let mut locks = writer_locks().lock().unwrap_or_else(|p| p.into_inner());
        locks.remove(&self.path);
    }
}

/// Claim the single-writer slot for `path` after capability validation.
pub fn claim_opfs_writer(path: &str, caps: &OpfsCapabilities) -> Result<OpfsWriterLease> {
    let mode = PersistentStorageMode::from_path(path)?;
    validate_opfs_capabilities(caps, mode)?;

    let mut locks = writer_locks().lock().unwrap_or_else(|p| p.into_inner());
    if locks.contains(path) {
        return Err(Error::LeaseUnavailable {
            key: path.to_string(),
        });
    }
    locks.insert(path.to_string());
    Ok(OpfsWriterLease {
        path: path.to_string(),
    })
}

/// Gate persistent OPFS open on capability contract, then host I/O (not implemented).
pub fn open_persistent(path: &str, caps: &OpfsCapabilities) -> Result<()> {
    let _lease = claim_opfs_writer(path, caps)?;
    Err(Error::Unsupported("OPFS host adapter I/O not implemented"))
}
