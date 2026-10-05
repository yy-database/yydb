use serde_json::{json, Value as JsonValue};
use crate::fixtures::{assert_ok_true, full_yydb_caps, parse_json, USER_SCHEMA};
use yydb_wasm::open_persistent_session;

#[test]
fn persistent_session_opens_and_runs_schema_check() {
    let session =
        open_persistent_session("session-app.yydb", &full_yydb_caps()).expect("open persistent session");
    assert_eq!(session.path(), "session-app.yydb");

    assert_ok_true(&session.ensure_schema(USER_SCHEMA));
}

#[test]
fn persistent_session_reopens_kv_after_close() {
    let path = "session-reopen.yydb";
    let caps = full_yydb_caps();

    let mut session = open_persistent_session(path, &caps).expect("open");
    assert_ok_true(&session.put("theme", b"dark"));
    session.close();

    let reopened = open_persistent_session(path, &caps).expect("reopen");
    let parsed = parse_json(&reopened.get("theme"));
    assert_eq!(parsed.get("ok"), Some(&JsonValue::Bool(true)));
    assert_eq!(parsed.get("value"), Some(&json!([100, 97, 114, 107])));
}
