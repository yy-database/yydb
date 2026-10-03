use yydb_server::assert_bind_allowed;

#[test]
fn loopback_bind_ok_without_insecure() {
    assert!(assert_bind_allowed("127.0.0.1:7700", false).is_ok());
    assert!(assert_bind_allowed("localhost:7700", false).is_ok());
}

#[test]
fn non_loopback_rejected_without_insecure() {
    assert!(assert_bind_allowed("0.0.0.0:7700", false).is_err());
    assert!(assert_bind_allowed("192.168.1.10:7700", false).is_err());
}

#[test]
fn non_loopback_allowed_with_insecure() {
    assert!(assert_bind_allowed("0.0.0.0:7700", true).is_ok());
}
