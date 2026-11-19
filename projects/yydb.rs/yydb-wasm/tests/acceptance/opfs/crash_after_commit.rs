// Crash after commit: a publish that returned must already be durable.
//
// The old fixture drove a fake in-memory publication. The same guarantee is now
// observed through the host byte store: publish through the binding, drop every
// handle, then read the volume back from a brand-new store.

use crate::fixtures::{USER_SCHEMA, assert_ok_true, cleanup, fs_store, temp_db};
use database::execution::engine::Connection;
use yydb_wasm::{PersistentVolume, open_persistent_session_with_store};

#[test]
fn yydb_opfs_crash_after_commit_keeps_the_committed_image() {
    let path = temp_db("crash-after-commit");
    cleanup(&path);

    {
        let mut session = open_persistent_session_with_store(path.to_str().unwrap(), fs_store(&path)).expect("open session");
        assert_ok_true(&session.ensure_schema(USER_SCHEMA));
        assert_ok_true(&session.put("kept", b"committed"));
        assert_ok_true(&session.put("second", b"row"));
        session.close().expect("close");
    }

    // Simulated crash: nothing survives except what the host already synced.
    let volume = PersistentVolume::open(path.to_str().unwrap(), fs_store(&path)).expect("reopen over a fresh store");
    assert!(volume.generation() >= 1, "a durable publish must have advanced the generation");

    let bytes = volume.read_main().expect("read main").expect("committed image must be readable after a crash");
    assert_eq!(&bytes[0..5], b"YDPG\0");
    assert_eq!(bytes.len() % 4096, 0, "published image must stay page aligned");

    let memory = Connection::open_in_memory().expect("memory");
    memory.load_main_snapshot(&bytes).expect("hydrate");
    assert_eq!(memory.get("kept").expect("get"), Some(b"committed".to_vec()));
    assert_eq!(memory.get("second").expect("get"), Some(b"row".to_vec()));

    cleanup(&path);
}

#[test]
fn yydb_opfs_unpublished_bytes_never_reach_the_volume() {
    let path = temp_db("crash-before-commit");
    cleanup(&path);

    let session = open_persistent_session_with_store(path.to_str().unwrap(), fs_store(&path)).expect("open session");
    assert!(!path.exists(), "opening an empty volume must not create a main file");
    drop(session);

    assert!(!path.exists(), "a session that never published must leave no main file behind");
    cleanup(&path);
}
