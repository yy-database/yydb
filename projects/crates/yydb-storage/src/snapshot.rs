//! Logical snapshot codec over the physical `YDPG` v0 pager.
//!
//! A [`Snapshot`] is opaque to this crate: metadata rows are caller-defined and
//! records are logical key/value rows. Only the physical partition between
//! reserved `__yydb/meta/` rows and user rows is known here.

use std::collections::BTreeMap;

use yydb_formats::{FilePager, MemoryPager, KEY_KIND_USER, TREE_RECORD};
use yydb_types::{Error, Result};

const META_PREFIX: &[u8] = b"__yydb/meta/";

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
    /// Delete one row, reporting whether it existed.
    fn delete_kv(&mut self, key: impl AsRef<[u8]>) -> Result<bool>;
    /// Scan every stored row with its physical key.
    fn scan_kv(&mut self) -> Result<Vec<(yydb_formats::TreeKey, Vec<u8>)>>;
}

impl PagerKv for FilePager {
    fn get_kv(&mut self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        FilePager::get_kv(self, key)
    }

    fn put_kv(&mut self, key: impl AsRef<[u8]>, value: &[u8]) -> Result<()> {
        FilePager::put_kv(self, key, value)
    }

    fn delete_kv(&mut self, key: impl AsRef<[u8]>) -> Result<bool> {
        FilePager::delete_kv(self, key)
    }

    fn scan_kv(&mut self) -> Result<Vec<(yydb_formats::TreeKey, Vec<u8>)>> {
        FilePager::scan_kv(self)
    }
}

impl PagerKv for MemoryPager {
    fn get_kv(&mut self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        MemoryPager::get_kv(self, key)
    }

    fn put_kv(&mut self, key: impl AsRef<[u8]>, value: &[u8]) -> Result<()> {
        MemoryPager::put_kv(self, key, value)
    }

    fn delete_kv(&mut self, key: impl AsRef<[u8]>) -> Result<bool> {
        MemoryPager::delete_kv(self, key)
    }

    fn scan_kv(&mut self) -> Result<Vec<(yydb_formats::TreeKey, Vec<u8>)>> {
        MemoryPager::scan_kv(self)
    }
}

/// Load a [`Snapshot`] from any pager.
pub fn read_snapshot<P: PagerKv>(pager: &mut P) -> Result<Snapshot> {
    let mut snapshot = Snapshot::default();
    for (key, value) in pager.scan_kv()? {
        if key.tree_id != TREE_RECORD || key.key_kind != KEY_KIND_USER {
            continue;
        }
        if key.encoded.starts_with(META_PREFIX) {
            snapshot.metadata.insert(key.encoded, value);
            continue;
        }
        let name = String::from_utf8(key.encoded)
            .map_err(|_| Error::Corrupt("record key is not UTF-8"))?;
        snapshot.records.insert(name, value);
    }
    Ok(snapshot)
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

fn read_rows(pager: &mut impl PagerKv) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut rows = BTreeMap::new();
    for (key, value) in pager.scan_kv()? {
        if key.tree_id != TREE_RECORD || key.key_kind != KEY_KIND_USER {
            continue;
        }
        if key.encoded.starts_with(META_PREFIX) {
            continue;
        }
        let name = String::from_utf8(key.encoded)
            .map_err(|_| Error::Corrupt("record key is not UTF-8"))?;
        rows.insert(name, value);
    }
    Ok(rows)
}
