// fixture: yydb.scenario.vos_transaction_backup

use crate::fixtures::yydb::{cleanup, open_temp_yydx, temp_yydx};
use yydb::Connection;

const USER_SCHEMA: &str = r#"
table User {
    @@user_id: uuid,
    user_name: utf8,
    active: bool,
}
"#;

const INSERT_ADA: &str = r#"
User {
    user_id: "550e8400-e29b-41d4-a716-446655440000",
    user_name: "ada",
    active: true,
}.insert()
"#;

#[test]
fn yydb_scenario_vos_transaction_backup() {
    let (conn, path) = open_temp_yydx("scenario-vos-txn");
    let backup = temp_yydx("scenario-vos-txn-copy");

    conn.ensure_schema(USER_SCHEMA).expect("schema");
    conn.begin().expect("begin");
    conn.execute(INSERT_ADA).expect("insert");
    conn.commit().expect("commit");

    let rows = conn
        .query(r#"User.filter(x => true).collect()"#)
        .expect("query");
    assert_eq!(rows.len(), 1);

    conn.backup_yydx_to(&backup).expect("backup");
    drop(conn);

    let restored = Connection::open_yydx(&backup).expect("open backup");
    let rows = restored
        .query(r#"User.filter(x => true).collect()"#)
        .expect("query after restore");
    assert_eq!(rows.len(), 1);

    cleanup(&path);
    cleanup(&backup);
}
