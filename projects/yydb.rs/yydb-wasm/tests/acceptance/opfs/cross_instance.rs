// Restart and cross-instance behaviour.
//
// A browser page reload, a worker restart and a second tab all amount to the
// same question: given the bytes that already reached the host, which rows are
// visible, and who is allowed to write? The temporary directory stands in for
// the OPFS backend so every reopen builds a brand-new store over the very same
// files.

use std::collections::HashSet;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use database::{
    storage::local_io::{FsVolumeStore, HostVolumeStore, VolumeFile},
    types::{Error, Result},
};
use yydb_wasm::open_persistent_session_with_store;

use crate::fixtures::{USER_SCHEMA, assert_ok_true, cleanup, parse_json, temp_db};

/// Store that refuses every write, standing in for a host that lost the device.
struct ReadOnlyStore {
    inner: FsVolumeStore,
}

impl HostVolumeStore for ReadOnlyStore {
    fn len(&mut self, file: &VolumeFile) -> Result<Option<u64>> {
        self.inner.len(file)
    }

    fn read_at(&mut self, file: &VolumeFile, offset: u64, buf: &mut [u8]) -> Result<usize> {
        self.inner.read_at(file, offset, buf)
    }

    fn write_at(&mut self, _file: &VolumeFile, _offset: u64, _body: &[u8]) -> Result<()> {
        Err(Error::Io(io::Error::other("the host is read-only")))
    }

    fn truncate(&mut self, _file: &VolumeFile, _len: u64) -> Result<()> {
        Err(Error::Io(io::Error::other("the host is read-only")))
    }

    fn sync(&mut self, _file: &VolumeFile) -> Result<()> {
        Err(Error::Io(io::Error::other("the host is read-only")))
    }

    fn remove(&mut self, _file: &VolumeFile) -> Result<()> {
        Err(Error::Io(io::Error::other("the host is read-only")))
    }
}

/// Writer lease registry shared by every store pointed at the same path.
///
/// OPFS derives the real lease from the single sync access handle. The registry
/// models exactly that: the second acquire of one path fails with
/// [`Error::Busy`] until the first holder drops its store.
#[derive(Clone, Default)]
struct LeaseRegistry {
    held: Arc<Mutex<HashSet<PathBuf>>>,
}

struct LeasedStore {
    inner: FsVolumeStore,
    path: PathBuf,
    registry: LeaseRegistry,
}

impl LeasedStore {
    fn open(path: &PathBuf, registry: &LeaseRegistry) -> Result<Self> {
        let mut held = registry.held.lock().unwrap();
        if !held.insert(path.clone()) {
            return Err(Error::Busy { path: path.display().to_string(), mode: "writer" });
        }
        Ok(Self { inner: FsVolumeStore::open(path), path: path.clone(), registry: registry.clone() })
    }
}

impl HostVolumeStore for LeasedStore {
    fn len(&mut self, file: &VolumeFile) -> Result<Option<u64>> {
        self.inner.len(file)
    }

    fn read_at(&mut self, file: &VolumeFile, offset: u64, buf: &mut [u8]) -> Result<usize> {
        self.inner.read_at(file, offset, buf)
    }

    fn write_at(&mut self, file: &VolumeFile, offset: u64, body: &[u8]) -> Result<()> {
        self.inner.write_at(file, offset, body)
    }

    fn truncate(&mut self, file: &VolumeFile, len: u64) -> Result<()> {
        self.inner.truncate(file, len)
    }

    fn sync(&mut self, file: &VolumeFile) -> Result<()> {
        self.inner.sync(file)
    }

    fn remove(&mut self, file: &VolumeFile) -> Result<()> {
        self.inner.remove(file)
    }
}

impl Drop for LeasedStore {
    fn drop(&mut self) {
        self.registry.held.lock().unwrap().remove(&self.path);
    }
}

#[test]
fn yydb_opfs_a_new_store_serves_the_committed_rows() {
    let path = temp_db("cross-reopen");
    cleanup(&path);

    {
        let mut first = open_persistent_session_with_store(path.to_str().unwrap(), Box::new(FsVolumeStore::open(&path)))
            .expect("open the first instance");
        assert_ok_true(&first.ensure_schema(USER_SCHEMA));
        assert_ok_true(&first.put("alpha", b"committed"));
        first.close();
    }

    // A brand-new store over the same bytes: what the host kept is what loads.
    let mut second = open_persistent_session_with_store(path.to_str().unwrap(), Box::new(FsVolumeStore::open(&path)))
        .expect("open a second instance");
    let parsed = parse_json(&second.get("alpha"));
    assert_eq!(parsed.get("ok"), Some(&serde_json::Value::Bool(true)), "a committed row must be visible: {parsed}");
    assert!(second.generation() >= 1.0, "the reopened instance must see the published generation");
    second.close();
    cleanup(&path);
}

#[test]
fn yydb_opfs_a_failed_publish_is_invisible_to_the_next_instance() {
    let path = temp_db("cross-uncommitted");
    cleanup(&path);

    {
        let mut committed = open_persistent_session_with_store(path.to_str().unwrap(), Box::new(FsVolumeStore::open(&path)))
            .expect("open");
        assert_ok_true(&committed.ensure_schema(USER_SCHEMA));
        assert_ok_true(&committed.put("alpha", b"committed"));
        committed.close();
    }

    // The host turns read-only after the committed publish. The row reaches the
    // in-memory engine but never the volume.
    {
        let mut blocked =
            open_persistent_session_with_store(path.to_str().unwrap(), Box::new(ReadOnlyStore { inner: FsVolumeStore::open(&path) }))
                .expect("open over a read-only host");
        let parsed = parse_json(&blocked.put("beta", b"never-durable"));
        assert_eq!(parsed.get("ok"), Some(&serde_json::Value::Bool(false)), "a failed publish must answer with an error: {parsed}");
        blocked.close();
    }

    let mut third = open_persistent_session_with_store(path.to_str().unwrap(), Box::new(FsVolumeStore::open(&path)))
        .expect("reopen after the failure");
    assert!(third.get("alpha").contains("\"ok\":true"));
    let beta = parse_json(&third.get("beta"));
    assert_eq!(beta.get("value"), Some(&serde_json::Value::Null), "an uncommitted row must not appear after a restart: {beta}");
    third.close();
    cleanup(&path);
}

#[test]
fn yydb_opfs_a_second_writer_on_the_same_lease_is_refused() {
    let path = temp_db("cross-lease");
    cleanup(&path);
    let registry = LeaseRegistry::default();

    let first = open_persistent_session_with_store(
        path.to_str().unwrap(),
        Box::new(LeasedStore::open(&path, &registry).expect("first lease")),
    )
    .expect("the first instance owns the lease");

    let error = match LeasedStore::open(&path, &registry) {
        Ok(_) => panic!("the second writer must not acquire a held lease"),
        Err(error) => error,
    };
    match error {
        Error::Busy { path: ref busy, mode } => {
            assert_eq!(busy.as_str(), path.display().to_string().as_str());
            assert_eq!(mode, "writer");
        }
        other => panic!("expected Busy, got {other:?}"),
    }

    // Releasing the first lease lets the next writer in, so the refusal is a
    // wait rather than a permanent lockout.
    drop(first);
    let mut second = open_persistent_session_with_store(
        path.to_str().unwrap(),
        Box::new(LeasedStore::open(&path, &registry).expect("the lease was released")),
    )
    .expect("the second writer opens after the release");
    assert_ok_true(&second.put("owner", b"second"));
    second.close();
    cleanup(&path);
}
