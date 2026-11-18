// Single writer: the exclusive handle is what makes concurrent writers impossible.
//
// OPFS guarantees one `FileSystemSyncAccessHandle` per file, so exclusivity is a
// host property rather than something the wasm binding can lease itself. When a
// host cannot assert it, the binding refuses the volume with an explicit
// capability error instead of pretending to hold a writer lease.

use database::types::Error;
use yydb_wasm::{OpfsCapabilities, PersistentStorageMode, validate_opfs_capabilities};

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
}

#[test]
fn yydb_opfs_single_writer_accepts_a_full_host_profile() {
    validate_opfs_capabilities(&OpfsCapabilities::host_sync_handles(), PersistentStorageMode::Yydb)
        .expect("an exclusive sync handle satisfies the single-writer contract");
}
