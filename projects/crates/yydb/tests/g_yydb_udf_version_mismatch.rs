// gate: G-YYDB-8
// fixture: yydb.udf.version_mismatch

mod common;

use common::{cleanup, open_temp_db};
use yydb::{Error, Value};

#[test]
fn g_yydb_udf_version_mismatch() {
    let (conn, path) = open_temp_db("udf-version-mismatch");
    conn.create_scalar_versioned("double", 1, 1, |args| match args {
        [Value::I64(n)] => Ok(Value::I64(n * 2)),
        _ => Err(Error::Udf {
            name: "double".into(),
            message: "expected i64".into(),
        }),
    })
    .unwrap();

    let err = conn
        .call_scalar_version("double", 2, &[Value::I64(3)])
        .unwrap_err();
    assert!(
        matches!(err, Error::UdfVersionMismatch { .. }),
        "gate G-YYDB-8 fixture yydb.udf.version_mismatch expected UdfVersionMismatch, got {err:?}"
    );
    cleanup(&path);
}
