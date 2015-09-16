//! Bulk insert integrity: every committed row must survive and stay readable.
//!
//! Regression probe for silent row loss at B-tree scale: sequential keys are
//! inserted in one transaction and every single one must be present afterwards.
//!
//! Kept Rust-side on purpose: this pins a page-split internal invariant of the
//! format layer (the defect originally surfaced here), and the host layer can
//! only observe it indirectly through slower end-to-end inserts.

use yydb::{Connection, Value};

const SCHEMA: &str = r#"
table Bench {
    @@id: utf8,
    payload: bytes,
}
"#;

fn key_at(index: usize) -> String {
    format!("key-{index:010}")
}

fn load(conn: &Connection, total: usize) {
    conn.begin().expect("begin");
    for index in 0..total {
        conn.execute_sql_write(
            "INSERT INTO Bench (id, payload) VALUES (?1, ?2)",
            &[Value::Text(key_at(index)), Value::Bytes(vec![(index % 251) as u8; 64])],
        )
        .unwrap_or_else(|error| panic!("insert `{}`: {error}", key_at(index)));
    }
    conn.commit().expect("commit");
}

fn missing_keys(conn: &Connection, total: usize) -> Vec<String> {
    let rows = conn.query_sql("SELECT id FROM Bench ORDER BY id").expect("scan");
    let mut present = std::collections::BTreeSet::new();
    for row in rows {
        if let Some(Value::Text(key)) = row.get("id") {
            present.insert(key.clone());
        }
    }
    (0..total).map(key_at).filter(|key| !present.contains(key)).collect()
}

#[test]
fn in_memory_bulk_inserts_keep_every_row() {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema(SCHEMA).expect("schema");
    let total = 2_000usize;
    load(&conn, total);
    let missing = missing_keys(&conn, total);
    assert!(missing.is_empty(), "lost {} rows, first missing: {:?}", missing.len(), &missing[..missing.len().min(8)]);
}

#[test]
fn delete_journal_bulk_inserts_keep_every_row() {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("yydb-bulk-integrity-del-{nonce}.yydb"));
    let conn = Connection::open_with_flags(&path, yydb::OpenFlags::new()).expect("open");
    conn.ensure_schema(SCHEMA).expect("schema");
    let total = 2_000usize;
    load(&conn, total);
    let missing = missing_keys(&conn, total);
    assert!(missing.is_empty(), "lost {} rows (delete journal), first missing: {:?}", missing.len(), &missing[..missing.len().min(8)]);
    conn.close();
    let _ = std::fs::remove_file(&path);
}

#[test]
fn durable_bulk_inserts_keep_every_row_across_reopen() {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("yydb-bulk-integrity-{nonce}.yydb"));
    let conn = Connection::open_with_flags(&path, yydb::OpenFlags::wal()).expect("open");
    conn.ensure_schema(SCHEMA).expect("schema");
    let total = 2_000usize;
    load(&conn, total);
    let missing = missing_keys(&conn, total);
    assert!(missing.is_empty(), "lost {} rows before reopen, first missing: {:?}", missing.len(), &missing[..missing.len().min(8)]);
    conn.close();
    drop(conn);

    let reopened = Connection::open_with_flags(&path, yydb::OpenFlags::wal()).expect("reopen");
    let missing = missing_keys(&reopened, total);
    assert!(missing.is_empty(), "lost {} rows after reopen, first missing: {:?}", missing.len(), &missing[..missing.len().min(8)]);
    reopened.close();
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(std::env::temp_dir().join(format!("yydb-bulk-integrity-{nonce}.yydb{suffix}")));
    }
}
