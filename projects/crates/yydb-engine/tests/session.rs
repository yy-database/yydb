//! Peer sessions over one durable store keep transaction and UDF state apart.

use std::fs;
use std::path::{Path, PathBuf};

use yydb_engine::{Connection, Result, Value};

fn temp_yydx(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "yydb-engine-session-{}-{}.yydx",
        std::process::id(),
        label
    ))
}

fn cleanup(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(format!("{}-wal", path.display()));
    let _ = fs::remove_file(format!("{}-shm", path.display()));
    let stem = path.file_stem().and_then(|name| name.to_str()).unwrap_or("app");
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let _ = fs::remove_dir_all(parent.join(format!("{stem}-objects")));
}

#[test]
fn sessions_share_records_and_isolate_transactions_and_udfs() -> Result<()> {
    let path = temp_yydx("isolation");
    cleanup(&path);
    let root = Connection::open(&path)?;
    let writer = root.session();
    let reader = root.session();

    writer.create_scalar("twice", 1, |args| match args {
        [Value::I64(value)] => Ok(Value::I64(value * 2)),
        _ => Err(yydb_engine::Error::Unsupported("twice expects one integer")),
    })?;
    assert_eq!(writer.list_scalars(), vec!["twice".to_owned()]);
    assert!(reader.list_scalars().is_empty());
    assert!(reader.call_scalar("twice", &[Value::I64(2)]).is_err());

    writer.put("shared", b"visible")?;
    assert_eq!(reader.get("shared")?, Some(b"visible".to_vec()));

    writer.begin()?;
    writer.put("pending", b"uncommitted")?;
    assert_eq!(reader.get("pending")?, None);
    writer.commit()?;
    assert_eq!(reader.get("pending")?, Some(b"uncommitted".to_vec()));

    reader.begin()?;
    reader.put("discarded", b"rollback")?;
    reader.rollback()?;
    assert_eq!(writer.get("discarded")?, None);

    drop(writer);
    drop(reader);
    drop(root);
    cleanup(&path);
    Ok(())
}
