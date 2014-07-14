// fixture: yydb.scenario.vos_transaction_backup

use crate::fixtures::yydb::{cleanup, open_temp_yydx, temp_yydx};
use yydb::Connection;

#[test]
fn yydb_scenario_vos_transaction_backup() {
    let (conn, path) = open_temp_yydx("scenario-vos-txn");
    let backup = temp_yydx("scenario-vos-txn-copy");

    conn.put("user/ada", b"ada").expect("put");

    assert_eq!(
        conn.get("user/ada").expect("get").as_deref(),
        Some(b"ada".as_slice())
    );

    conn.backup_yydx_to(&backup).expect("backup");
    drop(conn);

    let restored = Connection::open_yydx(&backup).expect("open backup");
    assert_eq!(
        restored
            .get("user/ada")
            .expect("get after restore")
            .as_deref(),
        Some(b"ada".as_slice())
    );

    cleanup(&path);
    cleanup(&backup);
}
