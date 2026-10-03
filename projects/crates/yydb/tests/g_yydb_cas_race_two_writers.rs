// gate: G-YYDB-2
// fixture: yydb.cas.race_two_writers

mod common;

use common::{cleanup, open_temp_db};

#[test]
fn g_yydb_cas_race_two_writers() {
    let (conn, path) = open_temp_db("cas-race-two-writers");
    conn.put("frontier/x", b"v0").unwrap();

    let first = conn
        .compare_exchange("frontier/x", Some(b"v0"), Some(b"winner"))
        .unwrap();
    let second = conn
        .compare_exchange("frontier/x", Some(b"v0"), Some(b"loser"))
        .unwrap();

    let wins = [first, second].iter().filter(|won| **won).count();
    assert_eq!(
        wins, 1,
        "gate G-YYDB-2 fixture yydb.cas.race_two_writers expected exactly one CAS winner"
    );
    assert_eq!(conn.get("frontier/x").unwrap(), Some(b"winner".to_vec()));
    cleanup(&path);
}
