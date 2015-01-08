//! End-to-end authorization tests — verify that dispatch gates honour
//! Role → Permission mappings for wire frames.

use yydb_engine::Connection;
use yydb_protocol::{
    Frame, MessageType, encode_execute_prepared, encode_kv_get, encode_kv_put, encode_prepare, encode_schema_ensure,
};
use yydb_server::{Principal, PrincipalKind, Role, Session, SessionState, dispatch};

/// A VOS schema document that yy-db accepts.
const VALID_SCHEMA: &str = "table User { @@id: i64, name: utf8, active: bool }";

/// Build a Principal with the given roles.
fn named_principal(name: &str, roles: Vec<Role>) -> Principal {
    Principal { name: name.to_owned(), kind: PrincipalKind::Service, roles }
}

/// Build a SessionState whose already-attached session carries the given principal.
fn state_with_principal(conn_id: u64, principal: Principal) -> SessionState {
    let state = SessionState::for_test_unauthed(conn_id);
    let now = std::time::SystemTime::now();
    let session = Session { principal, token: "t-test".to_owned(), created_at: now, expires_at: now + Session::TTL };
    *state.session.lock().unwrap() = Some(session);
    state
}

/// Whether a response frame is an AuthInsufficientPermission error.
fn is_permission_error(frame: &Frame) -> bool {
    if frame.msg_type != MessageType::Error {
        return false;
    }
    match serde_json::from_slice::<serde_json::Value>(&frame.body) {
        Ok(v) => v.get("code").and_then(|c| c.as_str()) == Some("yydb.auth.insufficient_permission"),
        Err(_) => false,
    }
}

/// Whether a response frame is any non-error.
fn is_ok(frame: &Frame) -> bool {
    !is_permission_error(frame) && frame.msg_type != MessageType::Error
}

// ─── Reader ──────────────────────────────────────────────────────────────

#[test]
fn reader_kv_put_is_denied() {
    let conn = Connection::open_in_memory().unwrap();
    let state = state_with_principal(1, named_principal("reader", vec![Role::Reader]));
    let req = Frame::new(MessageType::KvPut, 42, encode_kv_put("k", b"v"));
    let resp = dispatch(&conn, &state, &req);
    assert!(is_permission_error(&resp), "Reader KvPut must be denied, got {:?}", resp.msg_type);
}

#[test]
fn reader_kv_get_is_allowed() {
    let conn = Connection::open_in_memory().unwrap();
    let state = state_with_principal(2, named_principal("reader", vec![Role::Reader]));
    let req = Frame::new(MessageType::KvGet, 42, encode_kv_get("k"));
    let resp = dispatch(&conn, &state, &req);
    assert!(is_ok(&resp), "Reader KvGet must be allowed, got {:?}", resp.msg_type);
}

#[test]
fn reader_schema_ensure_is_denied() {
    let conn = Connection::open_in_memory().unwrap();
    let state = state_with_principal(3, named_principal("reader", vec![Role::Reader]));
    let req = Frame::new(MessageType::SchemaEnsure, 42, encode_schema_ensure(1, VALID_SCHEMA));
    let resp = dispatch(&conn, &state, &req);
    assert!(is_permission_error(&resp), "Reader SchemaEnsure must be denied, got {:?}", resp.msg_type);
}

// ─── Writer ──────────────────────────────────────────────────────────────

#[test]
fn writer_kv_put_is_allowed() {
    let conn = Connection::open_in_memory().unwrap();
    let state = state_with_principal(4, named_principal("writer", vec![Role::Writer]));
    let req = Frame::new(MessageType::KvPut, 42, encode_kv_put("k", b"v"));
    let resp = dispatch(&conn, &state, &req);
    assert!(is_ok(&resp), "Writer KvPut must be allowed, got {:?}", resp.msg_type);
}

#[test]
fn writer_kv_get_is_allowed() {
    let conn = Connection::open_in_memory().unwrap();
    let state = state_with_principal(5, named_principal("writer", vec![Role::Writer]));
    let req = Frame::new(MessageType::KvGet, 42, encode_kv_get("k"));
    let resp = dispatch(&conn, &state, &req);
    assert!(is_ok(&resp), "Writer KvGet must be allowed, got {:?}", resp.msg_type);
}

#[test]
fn writer_schema_ensure_is_denied() {
    let conn = Connection::open_in_memory().unwrap();
    let state = state_with_principal(6, named_principal("writer", vec![Role::Writer]));
    let req = Frame::new(MessageType::SchemaEnsure, 42, encode_schema_ensure(1, VALID_SCHEMA));
    let resp = dispatch(&conn, &state, &req);
    assert!(is_permission_error(&resp), "Writer SchemaEnsure must be denied, got {:?}", resp.msg_type);
}

// ─── SchemaAdmin ─────────────────────────────────────────────────────────

#[test]
fn schema_admin_kv_put_is_allowed() {
    let conn = Connection::open_in_memory().unwrap();
    let state = state_with_principal(7, named_principal("admin", vec![Role::SchemaAdmin]));
    let req = Frame::new(MessageType::KvPut, 42, encode_kv_put("k", b"v"));
    let resp = dispatch(&conn, &state, &req);
    assert!(is_ok(&resp), "SchemaAdmin KvPut must be allowed, got {:?}", resp.msg_type);
}

#[test]
fn schema_admin_kv_get_is_allowed() {
    let conn = Connection::open_in_memory().unwrap();
    let state = state_with_principal(8, named_principal("admin", vec![Role::SchemaAdmin]));
    let req = Frame::new(MessageType::KvGet, 42, encode_kv_get("k"));
    let resp = dispatch(&conn, &state, &req);
    assert!(is_ok(&resp), "SchemaAdmin KvGet must be allowed, got {:?}", resp.msg_type);
}

#[test]
fn schema_admin_schema_ensure_is_allowed() {
    let conn = Connection::open_in_memory().unwrap();
    let state = state_with_principal(9, named_principal("admin", vec![Role::SchemaAdmin]));
    let req = Frame::new(MessageType::SchemaEnsure, 42, encode_schema_ensure(1, VALID_SCHEMA));
    let resp = dispatch(&conn, &state, &req);
    assert!(is_ok(&resp), "SchemaAdmin SchemaEnsure must be allowed, got {:?}", resp.msg_type);
}

// ─── Info (read-only for everyone) ──────────────────────────────────────

#[test]
fn reader_info_is_allowed() {
    let conn = Connection::open_in_memory().unwrap();
    let state = state_with_principal(10, named_principal("reader", vec![Role::Reader]));
    let req = Frame::new(MessageType::Info, 42, Vec::new());
    let resp = dispatch(&conn, &state, &req);
    assert!(is_ok(&resp), "Reader Info must be allowed, got {:?}", resp.msg_type);
}

// ─── Multi-role (Reader + Writer == Writer effective) ────────────────────

#[test]
fn multi_role_reader_writer_can_write() {
    let conn = Connection::open_in_memory().unwrap();
    let state = state_with_principal(11, named_principal("svc", vec![Role::Reader, Role::Writer]));
    let req = Frame::new(MessageType::KvPut, 42, encode_kv_put("k", b"v"));
    let resp = dispatch(&conn, &state, &req);
    assert!(is_ok(&resp), "Reader+Writer must be able to write, got {:?}", resp.msg_type);
}

// ─── LoopbackOnly still full access (control) ───────────────────────────

#[test]
fn loopback_only_full_access() {
    let conn = Connection::open_in_memory().unwrap();
    let principal = Principal { name: String::new(), kind: PrincipalKind::Anonymous, roles: vec![Role::LoopbackOnly] };
    let state = state_with_principal(12, principal);

    let put = Frame::new(MessageType::KvPut, 42, encode_kv_put("k", b"v"));
    assert!(is_ok(&dispatch(&conn, &state, &put)), "LoopbackOnly KvPut");

    let ensure = Frame::new(MessageType::SchemaEnsure, 42, encode_schema_ensure(1, VALID_SCHEMA));
    assert!(is_ok(&dispatch(&conn, &state, &ensure)), "LoopbackOnly SchemaEnsure");
}

// ─── Permission helper sanity ────────────────────────────────────────────

#[test]
fn permission_as_str_is_stable() {
    use yydb_server::Permission;
    assert_eq!(Permission::Read.as_str(), "read");
    assert_eq!(Permission::Write.as_str(), "write");
    assert_eq!(Permission::Schema.as_str(), "schema");
}

#[test]
fn principal_has_permission_respects_roles() {
    let reader = named_principal("r", vec![Role::Reader]);
    let writer = named_principal("w", vec![Role::Writer]);
    let admin = named_principal("a", vec![Role::SchemaAdmin]);

    use yydb_server::Permission;

    assert!(reader.has_permission(Permission::Read));
    assert!(!reader.has_permission(Permission::Write));
    assert!(!reader.has_permission(Permission::Schema));

    assert!(writer.has_permission(Permission::Read));
    assert!(writer.has_permission(Permission::Write));
    assert!(!writer.has_permission(Permission::Schema));

    assert!(admin.has_permission(Permission::Read));
    assert!(admin.has_permission(Permission::Write));
    assert!(admin.has_permission(Permission::Schema));
}

// ─── Prepared statement dynamic gating ──────────────────────────────────

/// Extract the statement_id from a PrepareOk frame body.
fn sid_from_prepare_ok(frame: &Frame) -> String {
    // body is: u32 len + utf8 bytes
    let len = u32::from_le_bytes([frame.body[0], frame.body[1], frame.body[2], frame.body[3]]) as usize;
    std::str::from_utf8(&frame.body[4..4 + len]).expect("valid utf8").to_owned()
}

#[test]
fn reader_execute_prepared_read_is_allowed() {
    let conn = Connection::open_in_memory().unwrap();
    conn.ensure_schema(VALID_SCHEMA).unwrap();
    let state = state_with_principal(20, named_principal("reader", vec![Role::Reader]));

    let prepare = Frame::new(MessageType::Prepare, 1, encode_prepare("User.all().collect()"));
    let sid_frame = dispatch(&conn, &state, &prepare);
    assert_eq!(sid_frame.msg_type, MessageType::PrepareOk, "prepare must succeed");
    let sid = sid_from_prepare_ok(&sid_frame);

    let exec = Frame::new(MessageType::ExecutePrepared, 2, encode_execute_prepared(&sid, None));
    let resp = dispatch(&conn, &state, &exec);
    assert!(is_ok(&resp), "Reader ExecutePrepared(read) must be allowed, got {:?}", resp.msg_type);
}

#[test]
fn reader_execute_prepared_write_is_denied() {
    let conn = Connection::open_in_memory().unwrap();
    conn.ensure_schema(VALID_SCHEMA).unwrap();
    let state = state_with_principal(21, named_principal("reader", vec![Role::Reader]));

    let prepare = Frame::new(MessageType::Prepare, 1, encode_prepare("User { name: \"x\" }.insert()"));
    let sid_frame = dispatch(&conn, &state, &prepare);
    assert_eq!(sid_frame.msg_type, MessageType::PrepareOk, "prepare write VOS must succeed");
    let sid = sid_from_prepare_ok(&sid_frame);

    let exec = Frame::new(MessageType::ExecutePrepared, 2, encode_execute_prepared(&sid, None));
    let resp = dispatch(&conn, &state, &exec);
    assert!(is_permission_error(&resp), "Reader ExecutePrepared(write) must be denied, got {:?}", resp.msg_type);
}

#[test]
fn writer_execute_prepared_write_is_allowed() {
    let conn = Connection::open_in_memory().unwrap();
    conn.ensure_schema(VALID_SCHEMA).unwrap();
    let state = state_with_principal(22, named_principal("writer", vec![Role::Writer]));

    let prepare = Frame::new(MessageType::Prepare, 1, encode_prepare("User { name: \"x\" }.insert()"));
    let sid_frame = dispatch(&conn, &state, &prepare);
    assert_eq!(sid_frame.msg_type, MessageType::PrepareOk, "prepare write VOS must succeed");
    let sid = sid_from_prepare_ok(&sid_frame);

    let exec = Frame::new(MessageType::ExecutePrepared, 2, encode_execute_prepared(&sid, None));
    let resp = dispatch(&conn, &state, &exec);
    // Even though the Write stmt's ops are short-circuited to empty (prepare
    // doesn't lower DML), the dispatch dynamic gate should still let it
    // through because Writer has Permission::Write.
    assert!(!is_permission_error(&resp), "Writer ExecutePrepared(write) must be allowed");
}
