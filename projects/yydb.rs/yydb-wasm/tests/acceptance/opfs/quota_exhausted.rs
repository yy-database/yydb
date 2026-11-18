// Host quota: the store reports `(used, limit)` and the upstream publisher
// refuses an oversized image without losing the committed one.
//
// Quota accounting is a host capability (`HostVolumeStore::quota`), so the
// acceptance case drives a store that reports it, not a private fake volume.

use database::{
    execution::engine::Connection,
    storage::local_io::{FsVolumeStore, HostVolumeStore, VolumeFile},
    types::{Error, Result},
};
use yydb_wasm::PersistentVolume;

use crate::fixtures::{cleanup, temp_db};

/// File-system store that also reports a quota the host would enforce.
struct QuotaStore {
    inner: FsVolumeStore,
    limit: u64,
}

impl QuotaStore {
    fn new(path: &std::path::Path, limit: u64) -> Self {
        Self { inner: FsVolumeStore::open(path), limit }
    }
}

impl HostVolumeStore for QuotaStore {
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
    fn quota(&mut self) -> Result<Option<(u64, u64)>> {
        let used = self.inner.len(&VolumeFile::Main)?.unwrap_or(0);
        Ok(Some((used, self.limit)))
    }
}

fn small_image() -> Vec<u8> {
    let conn = Connection::open_in_memory().expect("memory");
    conn.put("seed", b"value").expect("seed write");
    conn.main_snapshot().expect("main snapshot")
}

fn grown_image() -> Vec<u8> {
    let conn = Connection::open_in_memory().expect("memory");
    conn.put("seed", b"value").expect("seed write");
    conn.put("bulk", &vec![7_u8; 64 * 1024]).expect("bulk write");
    conn.main_snapshot().expect("main snapshot")
}

#[test]
fn yydb_opfs_quota_exhausted_keeps_the_committed_generation() {
    let path = temp_db("quota-exhausted");
    cleanup(&path);

    let committed = small_image();
    let limit = committed.len() as u64;

    let volume = PersistentVolume::open(path.to_str().unwrap(), Box::new(QuotaStore::new(&path, limit))).expect("open");
    volume.publish_main(&committed).expect("first publish fits the quota");
    let generation = volume.generation();

    let oversized = grown_image();
    assert!(oversized.len() as u64 > limit, "fixture must grow past the quota");

    let error = volume.publish_main(&oversized).expect_err("publish past the quota must fail");
    assert!(matches!(error, Error::QuotaExceeded { .. }), "expected QuotaExceeded, got {error}");

    assert_eq!(volume.generation(), generation, "a refused publish must not advance the generation");
    assert_eq!(volume.read_main().expect("read main"), Some(committed), "committed image must stay readable");

    cleanup(&path);
}

#[test]
fn yydb_opfs_quota_exhausted_allows_small_publish() {
    let path = temp_db("quota-exhausted-small");
    cleanup(&path);

    let committed = small_image();
    let limit = committed.len() as u64;
    let volume = PersistentVolume::open(path.to_str().unwrap(), Box::new(QuotaStore::new(&path, limit))).expect("open");

    volume.publish_main(&committed).expect("first publish");
    volume.publish_main(&committed).expect("a same-sized publish still fits");

    cleanup(&path);
}
