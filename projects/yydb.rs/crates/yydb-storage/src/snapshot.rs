//! Logical snapshot codec over the physical `YDPG` v0 pager.
//!
//! A [`Snapshot`] is opaque to this crate: metadata rows are caller-defined and
//! records are logical key/value rows. Only the physical partition between
//! reserved `__yydb/meta/` rows and user rows is known here.

use std::collections::BTreeMap;

use yydb_formats::{FilePager, KEY_KIND_USER, MemoryPager, TREE_RECORD};
use yydb_types::{Error, Result};

const META_PREFIX: &[u8] = b"__yydb/meta/";
const META_PREFIX_TEXT: &str = "__yydb/meta/";
/// Smallest key above every `__yydb/meta/` key: `'0'` (0x30) follows the
/// separator `'/'` (0x2F), so the prefix range ends before it.
const META_END_TEXT: &str = "__yydb/meta0";

/// Durable database image handed between storage and the engine.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Reserved `__yydb/meta/` rows; values are encoded by the caller.
    pub metadata: BTreeMap<Vec<u8>, Vec<u8>>,
    /// Logical user records keyed by record name.
    pub records: BTreeMap<String, Vec<u8>>,
}

/// Key/value pager surface shared by file and in-memory pagers.
pub trait PagerKv {
    /// Read one reserved or user row.
    fn get_kv(&mut self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>>;
    /// Insert or replace one row.
    fn put_kv(&mut self, key: impl AsRef<[u8]>, value: &[u8]) -> Result<()>;
    /// Insert or replace several rows as one record-tree mutation.
    fn put_kv_batch(&mut self, rows: Vec<(Vec<u8>, Vec<u8>)>) -> Result<()> {
        for (key, value) in rows {
            self.put_kv(key, &value)?;
        }
        Ok(())
    }
    /// Delete one row, reporting whether it existed.
    fn delete_kv(&mut self, key: impl AsRef<[u8]>) -> Result<bool>;
    /// Scan every stored row with its physical key.
    fn scan_kv(&mut self) -> Result<Vec<(yydb_formats::TreeKey, Vec<u8>)>>;
    /// Scan rows whose user key lies in `[start, end)`. `end = None` means
    /// "to the last key".
    fn scan_kv_range(&mut self, start: &str, end: Option<&str>) -> Result<Vec<(yydb_formats::TreeKey, Vec<u8>)>>;
}

impl PagerKv for FilePager {
    fn get_kv(&mut self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        FilePager::get_kv(self, key)
    }

    fn put_kv(&mut self, key: impl AsRef<[u8]>, value: &[u8]) -> Result<()> {
        FilePager::put_kv(self, key, value)
    }

    fn put_kv_batch(&mut self, rows: Vec<(Vec<u8>, Vec<u8>)>) -> Result<()> {
        FilePager::put_kv_batch(self, rows)
    }

    fn delete_kv(&mut self, key: impl AsRef<[u8]>) -> Result<bool> {
        FilePager::delete_kv(self, key)
    }

    fn scan_kv(&mut self) -> Result<Vec<(yydb_formats::TreeKey, Vec<u8>)>> {
        FilePager::scan_kv(self)
    }

    fn scan_kv_range(&mut self, start: &str, end: Option<&str>) -> Result<Vec<(yydb_formats::TreeKey, Vec<u8>)>> {
        FilePager::scan_kv_range(self, start, end.map(str::as_bytes))
    }
}

impl PagerKv for MemoryPager {
    fn get_kv(&mut self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        MemoryPager::get_kv(self, key)
    }

    fn put_kv(&mut self, key: impl AsRef<[u8]>, value: &[u8]) -> Result<()> {
        MemoryPager::put_kv(self, key, value)
    }

    fn put_kv_batch(&mut self, rows: Vec<(Vec<u8>, Vec<u8>)>) -> Result<()> {
        MemoryPager::put_kv_batch(self, rows)
    }

    fn delete_kv(&mut self, key: impl AsRef<[u8]>) -> Result<bool> {
        MemoryPager::delete_kv(self, key)
    }

    fn scan_kv(&mut self) -> Result<Vec<(yydb_formats::TreeKey, Vec<u8>)>> {
        MemoryPager::scan_kv(self)
    }

    fn scan_kv_range(&mut self, start: &str, end: Option<&str>) -> Result<Vec<(yydb_formats::TreeKey, Vec<u8>)>> {
        MemoryPager::scan_kv_range(self, start, end.map(str::as_bytes))
    }
}

/// Load a [`Snapshot`] from any pager.
pub fn read_snapshot<P: PagerKv>(pager: &mut P) -> Result<Snapshot> {
    let mut snapshot = Snapshot::default();
    for (key, value) in pager.scan_kv()? {
        insert_row(&mut snapshot, key, value)?;
    }
    Ok(snapshot)
}

/// Half-open user-record key window `[start, end)`.
///
/// Record keys are the physical `__yydb/v0/row/<table>/<pk>` names, so a window
/// over one table's primary-key range lets the record tree descend straight to
/// the leaves that can hold the rows of one statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordWindow {
    /// Inclusive lower record name.
    pub start: String,
    /// Exclusive upper record name. `None` reads to the last key.
    pub end: Option<String>,
}

/// Load a [`Snapshot`] whose user records are confined to `windows`.
///
/// Reserved metadata rows are always read in full: they are few and the engine
/// needs the schema and contract for every statement. Callers own the rule that
/// the windows cover every row a plan can read, and an empty window list means
/// "metadata only".
pub fn read_snapshot_windows<P: PagerKv>(pager: &mut P, windows: &[RecordWindow]) -> Result<Snapshot> {
    let mut snapshot = Snapshot::default();
    for (key, value) in pager.scan_kv_range(META_PREFIX_TEXT, Some(META_END_TEXT))? {
        insert_row(&mut snapshot, key, value)?;
    }
    for window in windows {
        for (key, value) in pager.scan_kv_range(&window.start, window.end.as_deref())? {
            insert_row(&mut snapshot, key, value)?;
        }
    }
    Ok(snapshot)
}

fn insert_row(snapshot: &mut Snapshot, key: yydb_formats::TreeKey, value: Vec<u8>) -> Result<()> {
    if key.tree_id != TREE_RECORD || key.key_kind != KEY_KIND_USER {
        return Ok(());
    }
    if key.encoded.starts_with(META_PREFIX) {
        snapshot.metadata.insert(key.encoded, value);
        return Ok(());
    }
    let name = String::from_utf8(key.encoded).map_err(|_| Error::Corrupt("record key is not UTF-8"))?;
    snapshot.records.insert(name, value);
    Ok(())
}

/// Persist a [`Snapshot`] through incremental btree mutations.
pub fn write_snapshot<P: PagerKv>(pager: &mut P, snapshot: &Snapshot) -> Result<()> {
    // Metadata rows are additive: callers own the codec and may not model every
    // reserved row in the database, so absence never means deletion.
    for (key, value) in &snapshot.metadata {
        pager.put_kv(key, value)?;
    }
    let existing = read_rows(pager)?;
    for (key, value) in &snapshot.records {
        match existing.get(key.as_str()) {
            Some(old) if old == value => {}
            _ => pager.put_kv(key.as_bytes(), value)?,
        }
    }
    for key in existing.keys() {
        if !snapshot.records.contains_key(key.as_str()) {
            pager.delete_kv(key.as_bytes())?;
        }
    }
    Ok(())
}

/// Exact record-level change set supplied by a caller that already knows which
/// keys its statement touched.
///
/// [`write_snapshot`] has to scan every stored record to discover what changed.
/// A caller that applied one keyed mutation in memory knows the change set
/// exactly, so [`write_delta`] publishes it without scanning.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RecordDelta {
    /// Records to insert or replace, with their new bytes.
    pub puts: Vec<(String, Vec<u8>)>,
    /// Record names to remove. A key that is already absent is a no-op.
    pub deletes: Vec<String>,
}

/// Persist a [`RecordDelta`] through incremental btree mutations.
///
/// Reserved metadata rows are rewritten as a group exactly like
/// [`write_snapshot`] does, because their codec belongs to the caller and
/// absence never means deletion. The delta owns every user record: it must
/// list each record the statement added, replaced or removed, and it must not
/// name reserved `__yydb/meta/` rows in `deletes`.
pub fn write_delta<P: PagerKv>(pager: &mut P, metadata: &BTreeMap<Vec<u8>, Vec<u8>>, delta: &RecordDelta) -> Result<()> {
    let mut rows: Vec<(Vec<u8>, Vec<u8>)> =
        metadata.iter().map(|(key, value)| (key.clone(), value.clone())).collect();
    rows.extend(delta.puts.iter().map(|(key, value)| (key.as_bytes().to_vec(), value.clone())));
    pager.put_kv_batch(rows)?;
    for key in &delta.deletes {
        pager.delete_kv(key.as_bytes())?;
    }
    Ok(())
}

fn read_rows(pager: &mut impl PagerKv) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut rows = BTreeMap::new();
    for (key, value) in pager.scan_kv()? {
        if key.tree_id != TREE_RECORD || key.key_kind != KEY_KIND_USER {
            continue;
        }
        if key.encoded.starts_with(META_PREFIX) {
            continue;
        }
        let name = String::from_utf8(key.encoded).map_err(|_| Error::Corrupt("record key is not UTF-8"))?;
        rows.insert(name, value);
    }
    Ok(rows)
}
