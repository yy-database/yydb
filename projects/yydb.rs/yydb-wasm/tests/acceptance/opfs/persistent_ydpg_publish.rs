// Persistent `.yydb` session publishes a real `YDPG` main file through the host store.

use crate::fixtures::{USER_SCHEMA, assert_ok_true, cleanup, fs_store, parse_json, temp_db};
use yydb_wasm::open_persistent_session_with_store;

#[test]
fn yydb_opfs_persistent_session_publishes_ydpg_main() {
    let path = temp_db("ydpg-publish");
    cleanup(&path);

    let mut session = open_persistent_session_with_store(path.to_str().unwrap(), fs_store(&path)).expect("open session");
    assert_ok_true(&session.ensure_schema(USER_SCHEMA));
    assert_ok_true(&session.put("theme", b"dark"));
    let generation = session.generation();
    assert!(generation >= 1.0, "a publish must advance the generation, got {generation}");
    session.close();

    let bytes = std::fs::read(&path).expect("main file must be on the host store");
    assert_eq!(&bytes[0..5], b"YDPG\0");
    assert_eq!(bytes.len() % 4096, 0, "published image must stay page aligned");

    let memory = database::execution::engine::Connection::open_in_memory().expect("memory");
    memory.load_main_snapshot(&bytes).expect("hydrate");
    assert_eq!(memory.get("theme").expect("get"), Some(b"dark".to_vec()));
    cleanup(&path);
}

#[test]
fn yydb_opfs_persistent_session_rejects_a_corrupt_volume_image() {
    let path = temp_db("ydpg-corrupt");
    cleanup(&path);
    std::fs::write(&path, b"not-a-ydpg-image").expect("seed corrupt main file");

    let error = match open_persistent_session_with_store(path.to_str().unwrap(), fs_store(&path)) {
        Ok(_) => panic!("a corrupt main file must not open"),
        Err(error) => error,
    };
    assert!(matches!(error, database::types::Error::Corrupt(_)), "expected Corrupt, got {error}");
    cleanup(&path);
}

#[test]
fn yydb_opfs_persistent_session_keeps_values_across_reopen() {
    let path = temp_db("ydpg-reopen");
    cleanup(&path);

    let mut first = open_persistent_session_with_store(path.to_str().unwrap(), fs_store(&path)).expect("open");
    assert_ok_true(&first.put("theme", b"light"));
    first.close();

    let second = open_persistent_session_with_store(path.to_str().unwrap(), fs_store(&path)).expect("reopen");
    let parsed = parse_json(&second.get("theme"));
    assert_eq!(parsed.get("ok"), Some(&serde_json::Value::Bool(true)));
    cleanup(&path);
}
