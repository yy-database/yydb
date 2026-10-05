// fixture: yydb.opfs.persistent_ydpg_publish

use yydb_wasm::{open_persistent, open_persistent_session};

use crate::fixtures::full_yydb_caps;

#[test]
fn yydb_opfs_persistent_session_publishes_ydpg_main() {
    let path = "session-ydpg.yydb";
    let caps = full_yydb_caps();

    let mut session = open_persistent_session(path, &caps).expect("open session");
    assert!(session.put("theme", b"dark").contains("\"ok\":true"));
    session.close();

    let volume = open_persistent(path, &caps).expect("open volume");
    let bytes = volume
        .read_main()
        .expect("read main")
        .expect("main bytes present");
    assert_eq!(&bytes[0..5], b"YDPG\x00");

    let memory = yydb::Connection::open_in_memory().expect("memory");
    memory.load_main_snapshot(&bytes).expect("hydrate");
    assert_eq!(
        memory.get("theme").expect("get"),
        Some(b"dark".to_vec())
    );
}
