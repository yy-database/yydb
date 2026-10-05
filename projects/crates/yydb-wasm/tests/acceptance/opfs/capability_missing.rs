// fixture: yydb.opfs.capability_missing

use yydb::Error;
use yydb_wasm::{open_persistent, OpfsCapabilities};

#[test]
fn yydb_opfs_capability_missing() {
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
