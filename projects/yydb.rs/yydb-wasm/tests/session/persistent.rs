use crate::fixtures::{USER_SCHEMA, assert_ok_true, cleanup, fs_store, parse_json, temp_db};
use serde_json::{Value as JsonValue, json};
use yydb_wasm::open_persistent_session_with_store;

#[test]
fn persistent_session_opens_and_runs_schema_check() {
    let path = temp_db("session-schema");
    cleanup(&path);
    let mut session = open_persistent_session_with_store(path.to_str().unwrap(), fs_store(&path)).expect("open persistent session");
    assert_eq!(session.path(), path.to_str().unwrap());

    assert_ok_true(&session.ensure_schema(USER_SCHEMA));
    cleanup(&path);
}

#[test]
fn persistent_session_reopens_kv_after_close() {
    let path = temp_db("session-reopen");
    cleanup(&path);

    let mut session = open_persistent_session_with_store(path.to_str().unwrap(), fs_store(&path)).expect("open");
    assert_ok_true(&session.put("theme", b"dark"));
    session.close().expect("close");

    let reopened = open_persistent_session_with_store(path.to_str().unwrap(), fs_store(&path)).expect("reopen");
    let parsed = parse_json(&reopened.get("theme"));
    assert_eq!(parsed.get("ok"), Some(&JsonValue::Bool(true)));
    assert_eq!(parsed.get("value"), Some(&json!([100, 97, 114, 107])));
    cleanup(&path);
}
