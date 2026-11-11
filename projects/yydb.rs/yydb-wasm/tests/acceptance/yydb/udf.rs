use database::{
    execution::engine::Connection,
    types::{Error, Value},
};

use super::TempDb;

fn register_double(conn: &Connection) {
    conn.create_scalar_versioned("double", 1, 1, |args| match args.first() {
        Some(Value::I64(value)) => Ok(Value::I64(value * 2)),
        _ => Ok(Value::Null),
    })
    .expect("register double v1");
}

// fixture: yydb.udf.deterministic
#[test]
fn yydb_udf_deterministic() {
    let dir = TempDb::new("udf-deterministic");
    let path = dir.yydb();
    {
        let conn = Connection::open(&path).expect("open yydb");
        register_double(&conn);
        assert_eq!(conn.call_scalar_version("double", 1, &[Value::I64(21)]).expect("call"), Value::I64(42));
        conn.close();
    }

    let conn = Connection::open(&path).expect("reopen yydb");
    register_double(&conn);
    assert_eq!(conn.call_scalar_version("double", 1, &[Value::I64(21)]).expect("call"), Value::I64(42));
}

// fixture: yydb.udf.version_mismatch
#[test]
fn yydb_udf_version_mismatch() {
    let dir = TempDb::new("udf-version-mismatch");
    let conn = Connection::open(dir.yydb()).expect("open yydb");
    register_double(&conn);

    let error = conn.call_scalar_version("double", 2, &[Value::I64(21)]).expect_err("missing version must fail");
    assert!(matches!(error, Error::UdfVersionMismatch { .. }), "expected UdfVersionMismatch, got {error:?}");
}
