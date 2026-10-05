// gate: G-OPFS-2
// fixture: yydb.opfs.single_writer_busy
//
// Living `08` / `fixtures/g-yydb/cases/yydb_opfs_single_writer_busy.json`.

use yydb::Error;
use yydb_wasm::{claim_opfs_writer, OpfsCapabilities};

#[test]
fn g_yydb_opfs_single_writer_busy() {
    let caps = OpfsCapabilities::full_yydb_profile();
    let first = claim_opfs_writer("app.yydb", &caps).expect("first writer");
    let busy = claim_opfs_writer("app.yydb", &caps).unwrap_err();
    assert!(
        matches!(busy, Error::LeaseUnavailable { .. }),
        "expected LeaseUnavailable, got {busy}"
    );
    drop(first);
    claim_opfs_writer("app.yydb", &caps).expect("writer after release");
}

#[test]
fn g_yydb_opfs_single_writer_busy_different_paths() {
    let caps = OpfsCapabilities::full_yydb_profile();
    let _a = claim_opfs_writer("alpha.yydb", &caps).expect("alpha");
    let _b = claim_opfs_writer("beta.yydb", &caps).expect("beta");
}
