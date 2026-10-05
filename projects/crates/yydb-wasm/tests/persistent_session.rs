use serde_json::Value as JsonValue;
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
