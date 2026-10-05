// fixture: yydb.opfs.wal_sidecar

use crate::fixtures::{full_yydb_caps, full_yydx_caps};
use yydb_wasm::open_persistent;

#[test]
fn yydb_opfs_wal_sidecar_roundtrip() {
    let path = "probe-wal-sidecar.yydb";
    let volume = open_persistent(path, &full_yydb_caps()).expect("persistent open");

    volume
        .publish_main(b"main-generation")
        .expect("publish main");
    volume.publish_wal(b"YYWL-sidecar").expect("publish wal");
    volume.publish_shm(b"YYSH-sidecar").expect("publish shm");

    assert_eq!(
        volume.read_main().expect("read main"),
        Some(b"main-generation".to_vec())
    );
    assert_eq!(
        volume.read_wal().expect("read wal"),
        Some(b"YYWL-sidecar".to_vec())
    );
    assert_eq!(
        volume.read_shm().expect("read shm"),
        Some(b"YYSH-sidecar".to_vec())
    );

    volume.remove_wal().expect("remove wal");
    volume.remove_shm().expect("remove shm");
    assert_eq!(volume.read_wal().expect("read wal"), None);
    assert_eq!(volume.read_shm().expect("read shm"), None);
    assert_eq!(
        volume.read_main().expect("read main"),
        Some(b"main-generation".to_vec())
    );
}

#[test]
fn yydb_opfs_yydx_wal_sidecar_independent_of_blob_root() {
    let path = "probe-wal-sidecar.yydx";
    let volume = open_persistent(path, &full_yydx_caps()).expect("persistent open");

    volume.publish_blob("blob-a", b"chunk").expect("publish blob");
    volume.publish_wal(b"YYWL-yydx").expect("publish wal");

    assert_eq!(
        volume.read_blob("blob-a").expect("read blob"),
        Some(b"chunk".to_vec())
    );
    assert_eq!(
        volume.read_wal().expect("read wal"),
        Some(b"YYWL-yydx".to_vec())
    );
}
