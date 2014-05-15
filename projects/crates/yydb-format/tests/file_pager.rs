use std::path::PathBuf;

use yydb_format::{parse_page0, FilePager, PAGE_MAGIC, PAGE_SIZE};

fn temp_db(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("yydb-format-{name}-{}.yydb", std::process::id()))
}

#[test]
fn file_pager_create_and_reopen() {
    let path = temp_db("create");
    let _ = std::fs::remove_file(&path);
    {
        let mut pager = FilePager::open(&path, false).unwrap();
        pager.put_kv("hello", b"world").unwrap();
        assert_eq!(pager.get_kv("hello").unwrap(), Some(b"world".to_vec()));
    }
    let mut reopened = FilePager::open(&path, false).unwrap();
    assert_eq!(reopened.get_kv("hello").unwrap(), Some(b"world".to_vec()));
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[0..5], PAGE_MAGIC);
    assert!(bytes.len() >= PAGE_SIZE * 2);
    parse_page0(&bytes[0..PAGE_SIZE]).unwrap();
    let _ = std::fs::remove_file(&path);
}

#[test]
fn file_pager_wal_reopen() {
    let path = temp_db("wal");
    let wal_path = {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push("-wal");
        PathBuf::from(sidecar)
    };
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&wal_path);
    {
        let mut pager = FilePager::open(&path, true).unwrap();
        pager.put_kv("durable", b"value").unwrap();
        assert!(wal_path.exists());
    }
    let mut reopened = FilePager::open(&path, true).unwrap();
    assert_eq!(reopened.get_kv("durable").unwrap(), Some(b"value".to_vec()));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&wal_path);
}

#[test]
fn file_pager_checkpoint_truncates_wal() {
    let path = temp_db("checkpoint");
    let wal_path = {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push("-wal");
        PathBuf::from(sidecar)
    };
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&wal_path);
    {
        let mut pager = FilePager::open(&path, true).unwrap();
        pager.put_kv("k", b"v").unwrap();
        assert!(wal_path.exists());
        pager.checkpoint().unwrap();
        assert!(!wal_path.exists());
    }
    let mut reopened = FilePager::open(&path, true).unwrap();
    assert_eq!(reopened.get_kv("k").unwrap(), Some(b"v".to_vec()));
    let _ = std::fs::remove_file(&path);
}
