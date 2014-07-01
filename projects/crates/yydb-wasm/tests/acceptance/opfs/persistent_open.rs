// OPFS logical I/O: persistent open + main-file atomic publish roundtrip.

use yydb_wasm::open_persistent;

use crate::fixtures::{full_yydb_caps, full_yydx_caps};

#[test]
fn yydb_opfs_persistent_open_main_roundtrip() {
    let path = "probe-app.yydb";
    let volume =
        open_persistent(path, &full_yydb_caps()).expect("persistent open");
    assert_eq!(volume.path(), path);
    assert!(volume.read_main().expect("read").is_none());

    volume
        .publish_main(b"generation-1")
        .expect("publish main generation");
    let body = volume
        .read_main()
        .expect("read")
        .expect("generation present");
    assert_eq!(body, b"generation-1");

    volume
        .publish_main(b"generation-2")
        .expect("publish next generation");
    assert_eq!(
        volume
            .read_main()
            .expect("read")
            .expect("generation present"),
        b"generation-2"
    );
}

#[test]
fn yydb_opfs_persistent_open_yydx_blob_roundtrip() {
    let path = "probe-app.yydx";
    let volume =
        open_persistent(path, &full_yydx_caps()).expect("persistent open");
    volume
        .publish_blob("blob-a", b"chunk-a")
        .expect("publish blob");
    assert_eq!(
        volume.read_blob("blob-a").expect("read"),
        Some(b"chunk-a".to_vec())
    );
}
