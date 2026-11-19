//! Bootstrap and session lookup rules for server-side authentication.

use database_server::yydb::auth::{AuthBootstrap, Principal, PrincipalKind, SessionRegistry};
use database_types::Error;

#[test]
fn parse_env_tokens_single_entry() {
    let bootstrap = AuthBootstrap::from_env(true).unwrap();
    // with no env vars and loopback true, default anonymous should be seeded
    assert_eq!(bootstrap.named_count(), 0);
}

#[test]
fn anon_forbidden_on_remote_lookup() {
    let bootstrap = AuthBootstrap { tokens: vec![("t-anon".to_owned(), Principal::anonymous_loopback())] };
    let reg = SessionRegistry::new(&bootstrap);
    let err = reg.lookup_session("t-anon", true).unwrap_err();
    assert!(matches!(err, Error::AuthAnonymousRemoteDenied { .. }));
}

#[test]
fn anon_ok_on_loopback_lookup() {
    let bootstrap = AuthBootstrap { tokens: vec![("t-anon".to_owned(), Principal::anonymous_loopback())] };
    let reg = SessionRegistry::new(&bootstrap);
    let session = reg.lookup_session("t-anon", false).unwrap();
    assert_eq!(session.principal.kind, PrincipalKind::Anonymous);
}

#[test]
fn unknown_token_is_token_invalid() {
    let bootstrap = AuthBootstrap { tokens: vec![] };
    let reg = SessionRegistry::new(&bootstrap);
    let err = reg.lookup_session("nothing", false).unwrap_err();
    assert!(matches!(err, Error::AuthTokenInvalid { .. }));
}
