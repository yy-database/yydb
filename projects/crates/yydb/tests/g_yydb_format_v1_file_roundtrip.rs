// gate: format-v1 (Living 09)
// fixture: format_v1.file_roundtrip

use std::path::PathBuf;

use yydb_format::{FilePager, PAGE_MAGIC};

#[test]
fn g_yydb_format_v1_file_roundtrip() {
    let path = std::env::temp_dir().join(format!(
        "g-yydb-format-v1-file-{}-{}.yydb",
        std::process::id(),
        "roundtrip"
    ));
    let _ = std::fs::remove_file(&path);
    {
        let mut pager = FilePager::open(&path, true).unwrap();
        pager.put_kv("gate/key", b"ok").unwrap();
    }
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[0..5], PAGE_MAGIC);
    let mut reopened = FilePager::open(&path, true).unwrap();
    assert_eq!(
        reopened.get_kv("gate/key").unwrap(),
        Some(b"ok".to_vec())
    );
    let _ = std::fs::remove_file(&path);
    let wal = PathBuf::from(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(wal);
}
