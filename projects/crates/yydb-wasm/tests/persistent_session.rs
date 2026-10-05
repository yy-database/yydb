use serde_json::{json, Value as JsonValue};
use yydb_wasm::{open_persistent_session, OpfsCapabilities};

#[test]
fn persistent_session_opens_and_runs_schema_check() {
    let session =
        open_persistent_session("session-app.yydb", &OpfsCapabilities::full_yydb_profile())
            .expect("open persistent session");
    assert_eq!(session.path(), "session-app.yydb");

    let payload = session.ensure_schema("table User { @@id: i64, name: utf8 }");
    let parsed: JsonValue = serde_json::from_str(&payload).expect("json");
    assert_eq!(parsed.get("ok"), Some(&JsonValue::Bool(true)));
}

#[test]
fn persistent_session_reopens_kv_after_close() {
    let path = "session-reopen.yydb";
    let caps = OpfsCapabilities::full_yydb_profile();

    let mut session = open_persistent_session(path, &caps).expect("open");
    let put_payload = session.put("theme", b"dark");
    let parsed: JsonValue = serde_json::from_str(&put_payload).expect("json");
    assert_eq!(parsed.get("ok"), Some(&JsonValue::Bool(true)));
    session.close();

    let reopened = open_persistent_session(path, &caps).expect("reopen");
    let get_payload = reopened.get("theme");
    let parsed: JsonValue = serde_json::from_str(&get_payload).expect("json");
    assert_eq!(parsed.get("ok"), Some(&JsonValue::Bool(true)));
    assert_eq!(parsed.get("value"), Some(&json!([100, 97, 114, 107])));
}
