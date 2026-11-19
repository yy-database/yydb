//! 从 `src/auth.rs` 迁出的测试。

use database_server::yydb::auth::{PrincipalKind, Role, parse_env_tokens};

#[test]
fn parse_explicit_env_multiple_entries() {
    let raw = "t1=alice:SchemaAdmin;t2=svc:Reader,Writer;t3=anonymous:LoopbackOnly";
    let entries = parse_env_tokens(raw).unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].0, "t1");
    assert_eq!(entries[0].1.name, "alice");
    assert_eq!(entries[0].1.kind, PrincipalKind::Service);
    assert!(entries[0].1.roles.contains(&Role::SchemaAdmin));
    assert_eq!(entries[2].1.kind, PrincipalKind::Anonymous);
    assert!(entries[2].1.roles.contains(&Role::LoopbackOnly));
}
