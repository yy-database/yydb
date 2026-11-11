use database::execution::engine::Connection;

use super::TempDb;

// fixture: yydb.cas.create_if_absent
#[test]
fn yydb_cas_create_if_absent() {
    let dir = TempDb::new("cas-create-if-absent");
    let conn = Connection::open(dir.yydb()).expect("open yydb");

    let first = conn.compare_exchange("lease/item", None, Some(b"new")).expect("first create");
    let second = conn.compare_exchange("lease/item", None, Some(b"again")).expect("second create");
    assert!(first, "create-if-absent must win on an empty key");
    assert!(!second, "create-if-absent must lose once the key exists");
    assert_eq!(conn.get("lease/item").expect("get"), Some(b"new".to_vec()));
}

// fixture: yydb.cas.race_two_writers
#[test]
fn yydb_cas_race_two_writers() {
    let dir = TempDb::new("cas-race-two-writers");
    let conn = Connection::open(dir.yydb()).expect("open yydb");
    conn.put("frontier/x", b"v0").expect("seed");

    let first = conn.compare_exchange("frontier/x", Some(b"v0"), Some(b"winner")).expect("writer one");
    let second = conn.compare_exchange("frontier/x", Some(b"v0"), Some(b"loser")).expect("writer two");
    assert_eq!([first, second].iter().filter(|won| **won).count(), 1, "exactly one writer must win");
    assert_eq!(conn.get("frontier/x").expect("get"), Some(b"winner".to_vec()));
}
