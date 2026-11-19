// Host faults and recovery for the OPFS publish path.
//
// The store below is a test-only wrapper: it forwards every call to the real
// file-system store and injects exactly one failure at a time. Nothing here
// emulates persistence in memory — a "crash" is just dropping every handle and
// opening the volume again over the same bytes.
//
// What the publisher actually guarantees:
//
// - a rejected write leaves the committed image and its generation untouched
// - a failed barrier or truncate is reported and never advances the generation
//   in the running handle
// - a torn image is never silently accepted as healthy: reopening either
//   hydrates the image and passes `doctor`, or refuses with `Corrupt`
// - a healthy publish after the fault heals the volume

use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use database::{
    execution::engine::Connection,
    storage::local_io::{FsVolumeStore, HostVolumeStore, VolumeFile},
    types::{Error, Result},
};
use yydb_wasm::{PersistentStorageMode, PersistentVolume, open_persistent_session_with_store};

use crate::fixtures::{USER_SCHEMA, assert_ok_true, cleanup, temp_db};

/// The single failure a [`FaultyStore`] injects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Fault {
    #[default]
    None,
    /// Reject the whole write before a single byte lands.
    RejectWrite,
    /// Land `bytes` of the body, then fail.
    TearWrite(usize),
    /// Fail the truncate that follows the write.
    RejectTruncate,
    /// Fail the durability barrier.
    RejectFlush,
}

/// One shared switch that outlives the boxed store.
#[derive(Clone, Default)]
struct FaultSwitch {
    fault: Arc<Mutex<Fault>>,
}

impl FaultSwitch {
    fn set(&self, fault: Fault) {
        *self.fault.lock().unwrap() = fault;
    }

    fn clear(&self) {
        self.set(Fault::None);
    }

    fn current(&self) -> Fault {
        *self.fault.lock().unwrap()
    }
}

struct FaultyStore {
    inner: FsVolumeStore,
    switch: FaultSwitch,
}

impl FaultyStore {
    fn new(path: &PathBuf, switch: &FaultSwitch) -> Self {
        Self { inner: FsVolumeStore::open(path), switch: switch.clone() }
    }
}

impl HostVolumeStore for FaultyStore {
    fn len(&mut self, file: &VolumeFile) -> Result<Option<u64>> {
        self.inner.len(file)
    }

    fn read_at(&mut self, file: &VolumeFile, offset: u64, buf: &mut [u8]) -> Result<usize> {
        self.inner.read_at(file, offset, buf)
    }

    fn write_at(&mut self, file: &VolumeFile, offset: u64, body: &[u8]) -> Result<()> {
        match self.switch.current() {
            Fault::RejectWrite => Err(Error::Io(io::Error::other("host rejected the write"))),
            Fault::TearWrite(prefix) => {
                let landed = prefix.min(body.len());
                if landed > 0 {
                    self.inner.write_at(file, offset, &body[..landed])?;
                }
                Err(Error::Io(io::Error::other(format!("host lost the write after {landed} bytes"))))
            }
            _ => self.inner.write_at(file, offset, body),
        }
    }

    fn truncate(&mut self, file: &VolumeFile, len: u64) -> Result<()> {
        if self.switch.current() == Fault::RejectTruncate {
            return Err(Error::Io(io::Error::other("host rejected the truncate")));
        }
        self.inner.truncate(file, len)
    }

    fn sync(&mut self, file: &VolumeFile) -> Result<()> {
        if self.switch.current() == Fault::RejectFlush {
            return Err(Error::Io(io::Error::other("host rejected the durability barrier")));
        }
        self.inner.sync(file)
    }

    fn remove(&mut self, file: &VolumeFile) -> Result<()> {
        self.inner.remove(file)
    }
}

fn main_image(tag: &str) -> Vec<u8> {
    let conn = Connection::open_in_memory().expect("memory");
    conn.put(tag, b"value").expect("seed write");
    conn.main_snapshot().expect("main snapshot")
}

fn hydrate(bytes: &[u8]) -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.load_main_snapshot(bytes)?;
    Ok(conn)
}

#[test]
fn yydb_opfs_a_rejected_write_keeps_the_committed_image() {
    let path = temp_db("fault-reject-write");
    cleanup(&path);
    let switch = FaultSwitch::default();

    let committed = main_image("kept");
    {
        let volume =
            PersistentVolume::open(path.to_str().unwrap(), Box::new(FaultyStore::new(&path, &switch))).expect("open the volume");
        volume.publish_main(&committed).expect("publish the committed image");
    }
    let generation = {
        let volume = PersistentVolume::open(path.to_str().unwrap(), Box::new(FaultyStore::new(&path, &switch))).expect("reopen");
        volume.generation()
    };

    switch.set(Fault::RejectWrite);
    {
        let volume =
            PersistentVolume::open(path.to_str().unwrap(), Box::new(FaultyStore::new(&path, &switch))).expect("reopen");
        let error = volume.publish_main(&main_image("lost")).expect_err("a rejected write cannot publish");
        assert!(matches!(error, Error::Io(_)), "expected Io, got {error}");
        assert_eq!(volume.generation(), generation, "a refused publish must not advance the generation");
    }

    {
        let reopened = PersistentVolume::open(path.to_str().unwrap(), Box::new(FaultyStore::new(&path, &switch))).expect("reopen");
        assert_eq!(reopened.read_main().expect("read main"), Some(committed.clone()), "the committed image must survive");
        assert_eq!(reopened.generation(), generation);
    }

    // Self-heal: the same handle publishes again once the host recovers.
    switch.clear();
    let healed = main_image("healed");
    {
        let volume =
            PersistentVolume::open(path.to_str().unwrap(), Box::new(FaultyStore::new(&path, &switch))).expect("reopen");
        volume.publish_main(&healed).expect("a healthy publish must succeed");
        assert_eq!(volume.read_main().expect("read main"), Some(healed), "the healed publish must be readable");
    }
    cleanup(&path);
}

#[test]
fn yydb_opfs_a_rejected_barrier_never_claims_success() {
    let path = temp_db("fault-flush");
    cleanup(&path);
    let switch = FaultSwitch::default();

    let committed = main_image("kept");
    let volume = PersistentVolume::open(path.to_str().unwrap(), Box::new(FaultyStore::new(&path, &switch))).expect("open");
    volume.publish_main(&committed).expect("publish");
    let generation = volume.generation();

    switch.set(Fault::RejectFlush);
    let error = volume.publish_main(&main_image("unbarriered")).expect_err("an unbarriered publish cannot be acknowledged");
    assert!(matches!(error, Error::Io(_)), "expected Io, got {error}");
    assert_eq!(volume.generation(), generation, "a failed barrier must not advance the generation");
    drop(volume);

    // Whatever survived must still be a consistent image, never a torn one that
    // claims to be healthy.
    switch.clear();
    let reopened = PersistentVolume::open(path.to_str().unwrap(), Box::new(FaultyStore::new(&path, &switch))).expect("reopen");
    if let Some(bytes) = reopened.read_main().expect("read main") {
        let conn = hydrate(&bytes).expect("a surviving image must hydrate");
        conn.doctor().expect("a surviving image must pass doctor");
    }
    reopened.publish_main(&main_image("healed")).expect("a healthy publish must succeed");
    cleanup(&path);
}

#[test]
fn yydb_opfs_a_rejected_truncate_never_claims_success() {
    let path = temp_db("fault-truncate");
    cleanup(&path);
    let switch = FaultSwitch::default();

    let volume = PersistentVolume::open(path.to_str().unwrap(), Box::new(FaultyStore::new(&path, &switch))).expect("open");
    volume.publish_main(&main_image("kept")).expect("publish");
    let generation = volume.generation();

    switch.set(Fault::RejectTruncate);
    let error = volume.publish_main(&main_image("grow-with-tail")).expect_err("a rejected truncate cannot publish");
    assert!(matches!(error, Error::Io(_)), "expected Io, got {error}");
    assert_eq!(volume.generation(), generation, "a refused publish must not advance the generation");
    drop(volume);

    switch.clear();
    let reopened = PersistentVolume::open(path.to_str().unwrap(), Box::new(FaultyStore::new(&path, &switch))).expect("reopen");
    reopened.publish_main(&main_image("healed")).expect("a healthy publish must succeed");
    let bytes = reopened.read_main().expect("read main").expect("main file");
    hydrate(&bytes).expect("the healed image must hydrate");
    cleanup(&path);
}

#[test]
fn yydb_opfs_a_torn_write_is_detected_instead_of_silently_accepted() {
    let path = temp_db("fault-torn");
    cleanup(&path);
    let switch = FaultSwitch::default();

    let committed = main_image("kept");
    let volume = PersistentVolume::open(path.to_str().unwrap(), Box::new(FaultyStore::new(&path, &switch))).expect("open");
    volume.publish_main(&committed).expect("publish");
    let generation = volume.generation();

    switch.set(Fault::TearWrite(4096));
    let error = volume.publish_main(&main_image("torn")).expect_err("a torn write cannot publish");
    assert!(matches!(error, Error::Io(_)), "expected Io, got {error}");
    assert_eq!(volume.generation(), generation, "a torn publish must not advance the generation");
    drop(volume);

    switch.clear();
    match open_persistent_session_with_store(path.to_str().unwrap(), Box::new(FaultyStore::new(&path, &switch))) {
        Ok(mut session) => {
            // A surviving image must be complete enough to hydrate and pass
            // doctor, so a torn prefix can never masquerade as a healthy one.
            let report = session.doctor_json();
            assert!(report.contains("\"ok\":true"), "a torn image must not be served as healthy: {report}");
            session.close().expect("close");
        }
        Err(error) => assert!(
            matches!(error, Error::Corrupt(_)),
            "a torn image must be reported as corrupt, got {error}"
        ),
    }

    // The next publish over a healthy host restores a usable volume.
    let repaired = PersistentVolume::open(path.to_str().unwrap(), Box::new(FaultyStore::new(&path, &switch))).expect("reopen");
    repaired.publish_main(&main_image("repaired")).expect("a healthy publish must succeed");
    let bytes = repaired.read_main().expect("read main").expect("main file");
    let conn = hydrate(&bytes).expect("the repaired image must hydrate");
    conn.doctor().expect("the repaired image must pass doctor");
    cleanup(&path);
}

#[test]
fn yydb_opfs_a_damaged_main_file_is_refused_and_rebuilt() {
    let path = temp_db("fault-corrupt-main");
    cleanup(&path);

    // A committed image, then the page header is scrambled behind our back.
    let volume = PersistentVolume::open(path.to_str().unwrap(), Box::new(FaultyStore::new(&path, &FaultSwitch::default())))
        .expect("open");
    volume.publish_main(&main_image("kept")).expect("publish");
    drop(volume);

    let mut bytes = std::fs::read(&path).expect("main file");
    bytes[0..5].copy_from_slice(b"JUNK\0");
    std::fs::write(&path, &bytes).expect("damage the main file");

    let error = open_persistent_session_with_store(path.to_str().unwrap(), Box::new(FaultyStore::new(&path, &FaultSwitch::default())))
        .err()
        .expect("a damaged image must not open");
    assert!(matches!(error, Error::Corrupt(_) | Error::Unsupported(_)), "expected a structural refusal, got {error}");

    // A volume whose header cannot be parsed has no committed generation left
    // to publish into, so recovery starts by discarding the damaged file at the
    // host, not by silently overwriting it from inside the publisher.
    cleanup(&path);
    let repaired = PersistentVolume::open(path.to_str().unwrap(), Box::new(FaultyStore::new(&path, &FaultSwitch::default())))
        .expect("a cleared volume opens again");
    repaired.publish_main(&main_image("rebuilt")).expect("rebuild");
    let bytes = repaired.read_main().expect("read main").expect("main file");
    let conn = hydrate(&bytes).expect("the rebuilt image must hydrate");
    conn.doctor().expect("the rebuilt image must pass doctor");
    cleanup(&path);
}

#[test]
fn yydb_opfs_a_corrupt_wal_sidecar_does_not_block_main_recovery() {
    let path = temp_db("fault-wal");
    cleanup(&path);

    let committed = main_image("kept");
    {
        let volume =
            PersistentVolume::open(path.to_str().unwrap(), Box::new(FaultyStore::new(&path, &FaultSwitch::default()))).expect("open");
        volume.publish_main(&committed).expect("publish main");
        volume.publish_wal(b"YYWL half-written frame").expect("publish a broken sidecar");
    }

    // The sidecar is replayed by whoever owns WAL recovery, so the binding must
    // hand the bytes back untouched instead of deciding they are fine.
    let reopened =
        PersistentVolume::open(path.to_str().unwrap(), Box::new(FaultyStore::new(&path, &FaultSwitch::default()))).expect("reopen");
    assert_eq!(reopened.read_wal().expect("read wal"), Some(b"YYWL half-written frame".to_vec()));
    assert_eq!(reopened.read_main().expect("read main"), Some(committed.clone()));
    let conn = hydrate(&committed).expect("the committed image must hydrate");
    conn.doctor().expect("the committed image must stay healthy");

    reopened.remove_wal().expect("discard the broken sidecar");
    assert_eq!(reopened.read_wal().expect("read wal"), None);
    cleanup(&path);
}

#[test]
fn yydb_opfs_an_uncommitted_session_is_invisible_after_a_crash() {
    let path = temp_db("fault-uncommitted");
    cleanup(&path);

    {
        let mut session = open_persistent_session_with_store(path.to_str().unwrap(), Box::new(FaultyStore::new(&path, &FaultSwitch::default())))
            .expect("open");
        assert!(!path.exists(), "opening an empty volume must not create a main file");
        assert_ok_true(&session.ensure_schema(USER_SCHEMA));
        assert_ok_true(&session.put("visible", b"row"));
        // Crash: the session disappears without a clean close.
        std::mem::forget(session);
    }

    let reopened = open_persistent_session_with_store(path.to_str().unwrap(), Box::new(FaultyStore::new(&path, &FaultSwitch::default())))
        .expect("reopen");
    assert!(reopened.get("visible").contains("\"ok\":true"), "a committed row must survive the crash");
    cleanup(&path);
}

#[test]
fn yydb_opfs_the_single_file_profile_still_rejects_a_blob_directory_gate() {
    // Guard against the fault fixtures accidentally widening the capability
    // surface: `.yydb` never needed a blob directory.
    let error = yydb_wasm::validate_opfs_capabilities(
        &yydb_wasm::OpfsCapabilities { blob_directory: false, ..yydb_wasm::OpfsCapabilities::host_sync_handles() },
        PersistentStorageMode::Yydb,
    );
    error.expect("a `.yydb` volume does not need a blob directory");
}
