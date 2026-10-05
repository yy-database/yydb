// gate: G-OPFS-1
// fixture: yydb.opfs.capability_missing
//
// Living `08` / `fixtures/g-yydb/cases/yydb_opfs_capability_missing.json`.

use yydb::Error;
use yydb_wasm::{open_persistent, OpfsCapabilities};

#[test]
fn g_yydb_opfs_capability_missing() {
    let caps = OpfsCapabilities {
        atomic_publish: false,
        ..OpfsCapabilities::full_yydb_profile()
    };
    let error = open_persistent("app.yydb", &caps).unwrap_err();
    assert!(matches!(error, Error::Unsupported(_)));
    assert!(
        error.to_string().contains("atomic_publish"),
        "expected atomic_publish in {error}"
    );
}
