use yydb_server::{AuthBootstrap, Principal, PrincipalKind, assert_bind_allowed};

fn anon_bootstrap() -> AuthBootstrap {
    AuthBootstrap { tokens: vec![("t-anon".to_owned(), Principal::anonymous_loopback())] }
}

fn named_bootstrap() -> AuthBootstrap {
    AuthBootstrap {
        tokens: vec![("t-svc".to_owned(), Principal {
            name: "svc-worker-07".to_owned(),
            kind: PrincipalKind::Service,
            roles: vec![yydb_server::Role::Writer],
        })],
    }
}

#[test]
fn loopback_bind_ok_without_insecure() {
    let anonymous = anon_bootstrap();
    assert!(assert_bind_allowed("127.0.0.1:7700", false, &anonymous).is_ok());
    assert!(assert_bind_allowed("localhost:7700", false, &anonymous).is_ok());
}

#[test]
fn non_loopback_rejected_without_named_principal() {
    // non-loopback bind with only anonymous principal must refuse, even with insecure flag
    let anonymous = anon_bootstrap();
    assert!(assert_bind_allowed("0.0.0.0:7700", false, &anonymous).is_err());
    assert!(assert_bind_allowed("0.0.0.0:7700", true, &anonymous).is_err());
    assert!(assert_bind_allowed("192.168.1.10:7700", false, &anonymous).is_err());
}

#[test]
fn non_loopback_allowed_with_named_principal() {
    let named = named_bootstrap();
    assert!(assert_bind_allowed("0.0.0.0:7700", false, &named).is_ok());
    assert!(assert_bind_allowed("0.0.0.0:7700", true, &named).is_ok());
}

#[test]
fn anonymous_loopback_bootstrap_from_env() {
    // loopback-only defaults to anonymous principal when YYDB_AUTH_TOKENS is unset
    let bootstrap = AuthBootstrap::from_env(true).unwrap();
    assert_eq!(bootstrap.named_count(), 0);
    assert_eq!(bootstrap.tokens.len(), 1);
    assert_eq!(bootstrap.tokens[0].1.kind, PrincipalKind::Anonymous);
}

#[test]
fn non_loopback_refuses_when_env_unset() {
    // non-loopback bind must not start without explicit tokens
    let err = AuthBootstrap::from_env(false).unwrap_err();
    assert!(matches!(err, yydb_types::Error::AuthConfigMissing { .. }));
}
