//! Explicit WAL durability contract and injectable append failures.

use std::{
    io,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
};

use yydb_formats::{
    PAGE_SIZE, WalDurability, WalSink, WalTail, WalWriter, committed_transactions, encode_wal_header, replay_wal_pages,
    scan_wal, truncate_wal_torn_tail,
};
use yydb_types::Error;

const DB_ID: [u8; 16] = [0x33; 16];

#[derive(Default)]
struct SinkState {
    bytes: Vec<u8>,
    append_calls: usize,
    syncs: usize,
    fail_on_call: Option<usize>,
    partial: usize,
    fail_sync: bool,
}

#[derive(Clone, Default)]
struct TestSink(Arc<Mutex<SinkState>>);

impl TestSink {
    fn state(&self) -> MutexGuard<'_, SinkState> {
        self.0.lock().expect("test sink state is not poisoned")
    }
}

impl WalSink for TestSink {
    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut state = self.state();
        let call = state.append_calls;
        state.append_calls += 1;
        if state.fail_on_call == Some(call) {
            let take = state.partial.min(bytes.len());
            state.bytes.extend_from_slice(&bytes[..take]);
            return Err(io::Error::new(io::ErrorKind::StorageFull, "injected disk full"));
        }
        state.bytes.extend_from_slice(bytes);
        Ok(())
    }

    fn sync(&mut self) -> io::Result<()> {
        let mut state = self.state();
        if state.fail_sync {
            return Err(io::Error::other("injected sync failure"));
        }
        state.syncs += 1;
        Ok(())
    }
}

fn temp_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("yydb-wal-durability-{}-{}.yydb-wal", std::process::id(), label))
}

fn page(byte: u8) -> Vec<u8> {
    vec![byte; PAGE_SIZE]
}

fn writer(sink: TestSink) -> (WalWriter, PathBuf) {
    let path = temp_path("writer");
    sink.state().bytes.extend_from_slice(&encode_wal_header(DB_ID, 0));
    (WalWriter::with_sink(&path, DB_ID, Box::new(sink)), path)
}

#[test]
fn wal_commit_syncs_once_before_reporting_lsn() {
    let sink = TestSink::default();
    let (mut writer, _path) = writer(sink.clone());
    assert_eq!(writer.durability(), WalDurability::SyncOnCommit);

    let commit_lsn = writer.append_commit(&[(1, page(0x5A))], &[]).unwrap();
    assert_eq!(commit_lsn, 3, "begin, page image and commit consume three lsns");

    let state = sink.state();
    assert_eq!(state.append_calls, 1, "one transaction must be appended as one buffer");
    assert_eq!(state.syncs, 1, "one commit must issue exactly one durability barrier");
    let scan = scan_wal(&state.bytes).unwrap();
    assert_eq!(scan.tail, WalTail::Clean);
    assert_eq!(committed_transactions(&yydb_formats::parse_wal(&state.bytes).unwrap()), 1);
}

#[test]
fn wal_sync_failure_is_not_a_reported_commit() {
    let sink = TestSink::default();
    sink.state().fail_sync = true;
    let (mut writer, _path) = writer(sink.clone());

    let error = writer.append_commit(&[(1, page(0x5A))], &[]).unwrap_err();
    assert!(matches!(error, Error::Io(ref io_error) if io_error.kind() == io::ErrorKind::Other));
    assert_eq!(sink.state().syncs, 0, "a failed sync must not count as a completed barrier");

    let poisoned = writer.append_commit(&[(1, page(0x5B))], &[]).unwrap_err();
    assert!(matches!(poisoned, Error::Corrupt("wal writer poisoned after a failed append")));
}

#[test]
fn wal_short_append_is_a_torn_tail() {
    let sink = TestSink::default();
    let (mut writer, _path) = writer(sink.clone());
    writer.append_commit(&[(1, page(0x11))], &[]).unwrap();

    {
        let mut state = sink.state();
        state.fail_on_call = Some(1);
        state.partial = 30;
    }
    let error = writer.append_commit(&[(2, page(0x22))], &[]).unwrap_err();
    assert!(matches!(error, Error::Io(ref io_error) if io_error.kind() == io::ErrorKind::StorageFull));

    let state = sink.state();
    let scan = scan_wal(&state.bytes).unwrap();
    assert!(matches!(scan.tail, WalTail::Torn { .. }), "a short write must leave a torn tail");
    assert_eq!(committed_transactions(&yydb_formats::parse_wal_recover(&state.bytes).unwrap()), 1);

    let mut pages = std::collections::BTreeMap::new();
    let wal = yydb_formats::parse_wal_recover(&state.bytes).unwrap();
    replay_wal_pages(&wal, &mut pages).unwrap();
    assert_eq!(pages.get(&1), Some(&page(0x11)));
    assert_eq!(pages.get(&2), None, "an incomplete transaction must not publish page 2");
}

#[test]
fn wal_disk_full_before_write_keeps_prior_commit() {
    let sink = TestSink::default();
    let (mut writer, _path) = writer(sink.clone());
    writer.append_commit(&[(1, page(0x11))], &[]).unwrap();
    let committed_len = sink.state().bytes.len();

    {
        let mut state = sink.state();
        state.fail_on_call = Some(1);
        state.partial = 0;
    }
    let error = writer.append_commit(&[(2, page(0x22))], &[]).unwrap_err();
    assert!(matches!(error, Error::Io(ref io_error) if io_error.kind() == io::ErrorKind::StorageFull));

    let state = sink.state();
    assert_eq!(state.bytes.len(), committed_len);
    let scan = scan_wal(&state.bytes).unwrap();
    assert_eq!(scan.tail, WalTail::Clean, "a rejected append must not leave bytes behind");
    assert_eq!(committed_transactions(&yydb_formats::parse_wal(&state.bytes).unwrap()), 1);
}

#[test]
fn wal_writer_rejects_append_after_torn_tail() {
    let path = temp_path("torn-open");
    let _ = std::fs::remove_file(&path);
    let mut bytes = encode_wal_header(DB_ID, 0);
    bytes.extend_from_slice(&encode_wal_header(DB_ID, 0)[..7]);
    std::fs::write(&path, &bytes).unwrap();

    let error = WalWriter::open(&path, DB_ID, 0).err().unwrap();
    assert!(matches!(error, Error::Corrupt("wal torn tail requires recovery before append")));

    assert_eq!(truncate_wal_torn_tail(&path).unwrap(), 7);
    let writer = WalWriter::open(&path, DB_ID, 0).unwrap();
    assert_eq!(writer.database_id(), DB_ID);

    let _ = std::fs::remove_file(&path);
}
