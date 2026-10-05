//! Cross-process writer lock on `{db}-lock`.

use std::fs;
use std::io::ErrorKind;

use yydb::{Connection, Error, OpenFlags};

fn temp_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "yydb-format-v0-lock-{}-{}.yydb",
        std::process::id(),
        label
    ))
}

fn lock_path(db: &std::path::Path) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}-lock", db.display()))
}

#[test]
fn yydb_format_v0_writer_lock_exclusive() {
    let path = temp_path("exclusive");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(lock_path(&path));

    let first = Connection::open_with_flags(&path, OpenFlags::new()).unwrap();
    assert!(lock_path(&path).exists());

    let second = Connection::open_with_flags(&path, OpenFlags::new());
    match second {
        Err(Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => {}
        Ok(_) => panic!("expected writer lock contention, second open succeeded"),
        Err(other) => panic!("expected WouldBlock writer lock error, got {other}"),
    }

    drop(first);
    Connection::open_with_flags(&path, OpenFlags::new()).unwrap();

    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(lock_path(&path));
}
