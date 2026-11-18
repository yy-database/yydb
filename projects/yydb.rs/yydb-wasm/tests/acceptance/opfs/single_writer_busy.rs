// Single writer: the exclusive handle is what makes concurrent writers impossible.
//
// OPFS guarantees one `FileSystemSyncAccessHandle` per file, so exclusivity is a
// host property rather than something the wasm binding can lease itself. The
// host reports the outcome of its acquire attempt:
//
// - it holds the handle          -> the volume opens
// - another context holds it     -> `Error::Busy`, retryable once released
// - it never offers exclusivity  -> `opfs.exclusive_writer` capability error
//
// The binding never invents a lease it was not given.

use database::types::Error;
use yydb_wasm::{OpfsCapabilities, PersistentStorageMode, WriterLease, validate_opfs_capabilities, validate_writer_lease};

fn capability_of(result: Result<(), Error>) -> String {
    match result {
        Err(Error::NativeCapabilityMissing { capability }) => capability.to_owned(),
        other => panic!("expected NativeCapabilityMissing, got {other:?}"),
    }
}

#[test]
fn yydb_opfs_single_writer_requires_an_exclusive_handle() {
    let caps = OpfsCapabilities { exclusive_writer: false, ..OpfsCapabilities::host_sync_handles() };
    assert_eq!(capability_of(validate_opfs_capabilities(&caps, PersistentStorageMode::Yydb)), "opfs.exclusive_writer");
    assert_eq!(capability_of(validate_writer_lease("app.yydb", WriterLease::Absent)), "opfs.exclusive_writer");
}

#[test]
fn yydb_opfs_single_writer_accepts_a_full_host_profile() {
    validate_opfs_capabilities(&OpfsCapabilities::host_sync_handles(), PersistentStorageMode::Yydb)
        .expect("an exclusive sync handle satisfies the single-writer contract");
    validate_writer_lease("app.yydb", WriterLease::Held).expect("a held handle satisfies the single-writer contract");
}

#[test]
fn yydb_opfs_single_writer_reports_a_busy_holder_as_retryable() {
    let error = validate_writer_lease("app.yydb", WriterLease::Busy).expect_err("a busy lease must not open the volume");
    match error {
        Error::Busy { ref path, mode } => {
            assert_eq!(path, "app.yydb");
            assert_eq!(mode, "writer");
        }
        other => panic!("expected Busy, got {other:?}"),
    }
}
