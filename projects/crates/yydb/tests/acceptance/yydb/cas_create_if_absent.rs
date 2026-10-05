// fixture: yydb.cas.create_if_absent

#[path = "../../fixtures/yydb/mod.rs"]
mod harness;
use harness::{cleanup, open_temp_db};

#[test]
fn yydb_cas_create_if_absent() {
    let (conn, path) = open_temp_db("cas-create-if-absent");
    assert!(conn
        .compare_exchange("lease/item", None, Some(b"new"))
        .unwrap());
    assert!(!conn
        .compare_exchange("lease/item", None, Some(b"again"))
        .unwrap());
    assert_eq!(conn.get("lease/item").unwrap(), Some(b"new".to_vec()));
    cleanup(&path);
}
