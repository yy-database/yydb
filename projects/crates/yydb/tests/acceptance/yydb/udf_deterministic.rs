// fixture: yydb.udf.deterministic

use crate::fixtures::yydb::{cleanup, open_temp_db, reopen};
use yydb::{Error, Value};

#[test]
fn yydb_udf_deterministic() {
    let (conn, path) = open_temp_db("udf-deterministic");
    conn.create_scalar("double", 1, |args| match args {
        [Value::I64(n)] => Ok(Value::I64(n * 2)),
        _ => Err(Error::Udf {
            name: "double".into(),
            message: "expected i64".into(),
        }),
    })
    .unwrap();
    assert_eq!(
        conn.call_scalar("double", &[Value::I64(21)]).unwrap(),
        Value::I64(42)
    );
    drop(conn);

    let reopened = reopen(&path);
    reopened
        .create_scalar("double", 1, |args| match args {
            [Value::I64(n)] => Ok(Value::I64(n * 2)),
            _ => Err(Error::Udf {
                name: "double".into(),
                message: "expected i64".into(),
            }),
        })
        .unwrap();
    assert_eq!(
        reopened.call_scalar("double", &[Value::I64(21)]).unwrap(),
        Value::I64(42),
        "fixture yydb.udf.deterministic cross-reopen mismatch"
    );
    cleanup(&path);
}
