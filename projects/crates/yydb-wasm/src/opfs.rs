//! OPFS host profile capability probe and persistent-open gate (Living `08`).
//!
//! Host-testable gates and an in-process logical I/O backend. Browser OPFS sync handles
//! will replace the default backend without changing error semantics.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use yydb::{DoctorIssue, DoctorReport, DoctorSeverity, Error, Result};

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

/// Open handle to a logical OPFS-backed database after capability and writer checks.
#[derive(Debug)]
pub struct OpfsPersistentVolume {
    path: String,
    mode: PersistentStorageMode,
    #[allow(dead_code)]
    lease: OpfsWriterLease,
}

impl OpfsPersistentVolume {
    /// Logical database path (for example `app.yydb` or `app.yydx`).
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Storage format selected from the path suffix.
    pub fn mode(&self) -> PersistentStorageMode {
        self.mode
    }

    /// Read the committed main-file generation, if any.
    pub fn read_main(&self) -> Result<Option<Vec<u8>>> {
        crate::opfs_io::read_logical_file(&self.path, "")
    }

    /// Atomically publish the next main-file generation and sync the logical root.
    pub fn publish_main(&self, body: &[u8]) -> Result<()> {
        crate::opfs_io::publish_logical_file_atomic(&self.path, "", body)?;
        crate::opfs_io::sync_logical_root(&self.path)?;
        Ok(())
    }

    /// Publish one immutable blob chunk (`.yydx` only).
    pub fn publish_blob(&self, blob_hash: &str, body: &[u8]) -> Result<()> {
        if self.mode != PersistentStorageMode::Yydx {
            return Err(Error::Unsupported(
                "blob publish requires a .yydx logical path",
            ));
        }
        crate::opfs_io::publish_blob_atomic(&self.path, blob_hash, body)?;
        crate::opfs_io::sync_logical_root(&self.path)?;
        Ok(())
    }

    /// Read a published blob chunk (`.yydx` only).
    pub fn read_blob(&self, blob_hash: &str) -> Result<Option<Vec<u8>>> {
        if self.mode != PersistentStorageMode::Yydx {
            return Err(Error::Unsupported(
                "blob read requires a .yydx logical path",
            ));
        }
        crate::opfs_io::read_blob(&self.path, blob_hash)
    }
}

/// Gate persistent OPFS open on capability contract, then attach logical I/O.
pub fn open_persistent(path: &str, caps: &OpfsCapabilities) -> Result<OpfsPersistentVolume> {
    let mode = PersistentStorageMode::from_path(path)?;
    validate_opfs_capabilities(caps, mode)?;
    let lease = claim_opfs_writer(path, caps)?;
    Ok(OpfsPersistentVolume {
        path: path.to_string(),
        mode,
        lease,
    })
}

/// In-memory committed-generation stub for OPFS gate tests (Living `08` §5).
///
/// Models atomic publish with quota accounting. Production OPFS I/O will replace this
/// surface without changing error semantics.
#[derive(Debug, Clone)]
pub struct OpfsCommittedVolume {
    path: String,
    committed: Vec<u8>,
    quota_limit: u64,
    quota_used: u64,
}

impl OpfsCommittedVolume {
    /// Open a logical OPFS volume with an initial committed generation and byte quota.
    pub fn new(path: &str, initial_generation: &[u8], quota_limit: u64) -> Self {
        Self {
            path: path.to_string(),
            committed: initial_generation.to_vec(),
            quota_limit,
            quota_used: initial_generation.len() as u64,
        }
    }

    /// Read the last successfully published generation.
    pub fn read_committed(&self) -> &[u8] {
        &self.committed
    }

    /// Atomically publish `body` as the next generation, or abort without mutation.
    pub fn publish_generation(&mut self, body: &[u8]) -> Result<()> {
        let next_used = self.quota_used + body.len() as u64;
        if next_used > self.quota_limit {
            return Err(Error::QuotaExceeded {
                namespace: format!("opfs:{path}", path = self.path),
            });
        }
        self.quota_used = next_used;
        self.committed = body.to_vec();
        Ok(())
    }
}

/// Models `.yydx` blob publish then catalog commit (Living `07` §13 steps 2–4).
#[derive(Debug, Clone)]
pub struct OpfsBlobPublication {
    /// Logical `.yydx` path for inventory tracking.
    path: String,
    /// Blobs published to the object store (immutable once visible on disk).
    published_blobs: HashSet<String>,
    /// Blob hashes referenced by the committed catalog generation.
    committed_refs: HashSet<String>,
}

impl OpfsBlobPublication {
    /// Open a logical `.yydx` volume with an initial committed blob set.
    pub fn new(path: &str, initial_committed_refs: &[&str]) -> Self {
        let committed_refs: HashSet<String> = initial_committed_refs
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let published_blobs = committed_refs.clone();
        for hash in &published_blobs {
            track_blob(path, hash);
        }
        Self {
            path: path.to_string(),
            published_blobs,
            committed_refs,
        }
    }

    /// Publish a blob chunk (step 2). Does not advance the committed catalog.
    pub fn publish_blob(&mut self, blob_hash: &str) {
        self.published_blobs.insert(blob_hash.to_string());
        track_blob(&self.path, blob_hash);
    }

    /// Commit catalog references (steps 3–4). Refuses unpublished blob hashes.
    pub fn commit_catalog(&mut self, refs: &[&str]) -> Result<()> {
        for hash in refs {
            if !self.published_blobs.contains(*hash) {
                return Err(Error::ObjectNotFound {
                    hash_hex: (*hash).to_string(),
                });
            }
        }
        self.committed_refs = refs.iter().map(|s| (*s).to_string()).collect();
        Ok(())
    }

    /// Query-visible references (committed catalog only).
    pub fn visible_references(&self) -> Vec<String> {
        let mut refs = self.committed_refs.iter().cloned().collect::<Vec<_>>();
        refs.sort();
        refs
    }

    /// Published blobs not yet referenced by the committed catalog (orphans after crash).
    pub fn orphan_blobs(&self) -> Vec<String> {
        let mut orphans = self
            .published_blobs
            .difference(&self.committed_refs)
            .cloned()
            .collect::<Vec<_>>();
        orphans.sort();
        orphans
    }

    /// Committed references without a published blob (forbidden dangling state).
    pub fn dangling_references(&self) -> Vec<String> {
        let mut dangling = self
            .committed_refs
            .difference(&self.published_blobs)
            .cloned()
            .collect::<Vec<_>>();
        dangling.sort();
        dangling
    }
}

fn durable_snapshots() -> &'static Mutex<HashMap<String, OpfsDurableSnapshot>> {
    static SNAPSHOTS: OnceLock<Mutex<HashMap<String, OpfsDurableSnapshot>>> = OnceLock::new();
    SNAPSHOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Durable committed catalog generation visible after reopen (Living `07` §13 step 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpfsDurableSnapshot {
    /// Monotonic committed generation for the logical database path.
    pub generation: u64,
    /// Committed catalog bytes (opaque stub payload).
    pub catalog_body: Vec<u8>,
    /// Blob hashes referenced by the committed catalog.
    pub committed_refs: HashSet<String>,
}

impl OpfsDurableSnapshot {
    /// Sorted query-visible blob references.
    pub fn visible_references(&self) -> Vec<String> {
        let mut refs = self.committed_refs.iter().cloned().collect::<Vec<_>>();
        refs.sort();
        refs
    }
}

/// Atomically commit `publication` and sync durable catalog state for `path`.
pub fn opfs_commit_and_sync(
    path: &str,
    publication: &mut OpfsBlobPublication,
    refs: &[&str],
    catalog_body: &[u8],
) -> Result<OpfsDurableSnapshot> {
    publication.commit_catalog(refs)?;
    let generation = opfs_reopen(path)
        .map(|snapshot| snapshot.generation + 1)
        .unwrap_or(1);
    let snapshot = OpfsDurableSnapshot {
        generation,
        catalog_body: catalog_body.to_vec(),
        committed_refs: refs.iter().map(|hash| (*hash).to_string()).collect(),
    };
    durable_snapshots()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(path.to_string(), snapshot.clone());
    Ok(snapshot)
}

/// Reopen durable committed state after a crash (`yydb.opfs.crash_after_commit`).
pub fn opfs_reopen(path: &str) -> Result<OpfsDurableSnapshot> {
    durable_snapshots()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(path)
        .cloned()
        .ok_or(Error::Unsupported("OPFS durable snapshot not found"))
}

fn blob_inventory() -> &'static Mutex<HashMap<String, HashSet<String>>> {
    static INVENTORY: OnceLock<Mutex<HashMap<String, HashSet<String>>>> = OnceLock::new();
    INVENTORY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn track_blob(path: &str, blob_hash: &str) {
    blob_inventory()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .entry(path.to_string())
        .or_default()
        .insert(blob_hash.to_string());
}

/// Simulate user or browser eviction of a published blob from OPFS storage.
pub fn opfs_evict_blob(path: &str, blob_hash: &str) {
    blob_inventory()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .entry(path.to_string())
        .and_modify(|blobs| {
            blobs.remove(blob_hash);
        });
}

/// Read-only OPFS consistency probe for committed catalog vs blob inventory (`yydb.opfs.doctor_missing_blob`).
pub fn opfs_doctor(path: &str) -> Result<DoctorReport> {
    let snapshot = opfs_reopen(path)?;
    let inventory = blob_inventory().lock().unwrap_or_else(|p| p.into_inner());
    let present = inventory.get(path);
    let mut issues = Vec::new();
    for hash in snapshot.visible_references() {
        let missing = present.is_none_or(|blobs| !blobs.contains(&hash));
        if missing {
            issues.push(DoctorIssue {
                severity: DoctorSeverity::Error,
                code: "yydb.doctor.missing_blob".into(),
                message: format!("committed catalog references missing blob `{hash}`"),
                key_hint: Some(hash),
            });
        }
    }
    Ok(DoctorReport {
        file_path: Some(path.to_string()),
        last_commit_sequence: snapshot.generation,
        last_checkpoint_sequence: snapshot.generation,
        checkpoint_lag_records: 0,
        wal_bytes_uncheckpointed: 0,
        orphan_object_count: 0,
        namespaces: Vec::new(),
        issues,
    })
}
