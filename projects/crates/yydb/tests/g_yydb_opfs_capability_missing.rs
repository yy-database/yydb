// gate: G-OPFS-1
// fixture: yydb.opfs.capability_missing
//
// Living `08` / `fixtures/g-yydb/cases/yydb_opfs_capability_missing.json`.
// OPFS persistent open is not implemented in yydb-wasm yet.

#[test]
#[ignore = "OPFS host adapter not implemented (Living 08)"]
fn g_yydb_opfs_capability_missing() {
    panic!("implement when yydb-wasm exposes persistent OPFS open with capability probe");
}
