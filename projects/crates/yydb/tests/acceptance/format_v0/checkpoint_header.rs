//! fixture: format_v0.checkpoint_header

use std::fs;

use yydb::{Connection, OpenFlags};
use yydb_format::parse_page0;

fn temp_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "yydb-format-v0-checkpoint-header-{}-{}.yydb",
        std::process::id(),
        label
    ))
}

#[test]
fn yydb_format_v0_checkpoint_header() {
    let path = temp_path("checkpoint");
    let _ = fs::remove_file(&path);

    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).unwrap();
    conn.put("k", b"v").unwrap();
    conn.checkpoint().unwrap();
    drop(conn);

    let bytes = fs::read(&path).unwrap();
    let header = parse_page0(&bytes).unwrap();
    assert!(header.slot.generation > 1);
    assert!(header.slot.checkpoint_lsn > 0);

    let _ = fs::remove_file(&path);
}
