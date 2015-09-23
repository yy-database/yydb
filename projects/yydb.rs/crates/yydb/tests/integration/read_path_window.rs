//! Narrowed-read parity for point and range reads.
//!
//! The engine reads only the record-key window a plan can reach and lets the
//! normal filter decide every row. These probes pin the half-open-window
//! semantics against a full materialization on every durability path: page
//! splits, the delete journal, the write-ahead journal, reopen, and single-file
//! embedded objects.

use std::collections::BTreeMap;

use yydb::{Connection, INLINE_BYTES_MAX, OpenFlags, Value};

const SCHEMA: &str = r#"
table Bench {
    @@id: utf8,
    payload: bytes,
}
"#;

const POINT: &str = "SELECT id, payload FROM Bench WHERE id = ?1";
const RANGE: &str = "SELECT id, payload FROM Bench WHERE id >= ?1 AND id < ?2 ORDER BY id";
const ALL: &str = "SELECT id, payload FROM Bench ORDER BY id";

const TOTAL: usize = 2_000;

fn key_at(index: usize) -> String {
    format!("key-{index:010}")
}

fn payload_at(index: usize) -> Vec<u8> {
    vec![(index % 251) as u8; 64]
}

fn load(conn: &Connection, total: usize) {
    conn.begin().expect("begin");
    for index in 0..total {
        conn.execute_sql_write(
            "INSERT INTO Bench (id, payload) VALUES (?1, ?2)",
            &[Value::Text(key_at(index)), Value::Bytes(payload_at(index))],
        )
        .unwrap_or_else(|error| panic!("insert `{}`: {error}", key_at(index)));
    }
    conn.commit().expect("commit");
}

fn rows_of(rows: Vec<BTreeMap<String, Value>>) -> Vec<(String, Vec<u8>)> {
    rows.into_iter()
        .map(|row| {
            let id = match row.get("id") {
                Some(Value::Text(id)) => id.clone(),
                other => panic!("unexpected id column: {other:?}"),
            };
            let payload = match row.get("payload") {
                Some(Value::Bytes(payload)) => payload.clone(),
                other => panic!("unexpected payload column for `{id}`: {other:?}"),
            };
            (id, payload)
        })
        .collect()
}

fn full_scan(conn: &Connection) -> Vec<(String, Vec<u8>)> {
    rows_of(conn.query_sql(ALL).expect("full scan"))
}

fn point(conn: &Connection, key: &str) -> Option<(String, Vec<u8>)> {
    rows_of(
        conn.query_sql_with_params(POINT, &[Value::Text(key.to_owned())])
            .unwrap_or_else(|error| panic!("point `{key}`: {error}")),
    )
    .into_iter()
    .next()
}

fn range(conn: &Connection, low: &str, high: &str) -> Vec<(String, Vec<u8>)> {
    rows_of(
        conn.query_sql_with_params(RANGE, &[Value::Text(low.to_owned()), Value::Text(high.to_owned())])
            .unwrap_or_else(|error| panic!("range `{low}`..`{high}`: {error}")),
    )
}

/// Compare every windowed read against the same slice of a full scan.
fn assert_parity(conn: &Connection, label: &str) {
    let full = full_scan(conn);
    assert_eq!(full.len(), TOTAL, "{label}: full scan must see every row");

    for index in [0, 1, 37, 999, 1_500, TOTAL - 1] {
        let key = key_at(index);
        let got = point(conn, &key);
        let want = full.iter().find(|(id, _)| *id == key).cloned();
        assert_eq!(got, want, "{label}: point `{key}`");
    }

    for missing in ["key-0000200000".to_string(), "key-9999999999".to_string(), "".to_string()] {
        assert_eq!(point(conn, &missing), None, "{label}: absent point `{missing}`");
    }

    let ranges = [
        (key_at(0), key_at(TOTAL)),
        (key_at(0), key_at(1)),
        (key_at(500), key_at(510)),
        (key_at(1_990), key_at(TOTAL)),
        (key_at(1_200), key_at(1_200)),
        (key_at(TOTAL), key_at(TOTAL + 100)),
        (key_at(TOTAL + 100), key_at(TOTAL + 200)),
        ("key-0000000000".to_string(), "key-0000000010".to_string()),
    ];
    for (low, high) in ranges {
        let got = range(conn, &low, &high);
        let want: Vec<(String, Vec<u8>)> =
            full.iter().filter(|(id, _)| id.as_str() >= low.as_str() && id.as_str() < high.as_str()).cloned().collect();
        assert_eq!(got, want, "{label}: range `{low}`..`{high}`");
    }
}

#[test]
fn delete_journal_reads_match_full_scan_after_page_splits() {
    let path = crate::fixtures::yydb::temp_db_path("read-window-delete");
    let conn = Connection::open_with_flags(&path, OpenFlags::new()).expect("open");
    conn.ensure_schema(SCHEMA).expect("schema");
    load(&conn, TOTAL);
    assert_parity(&conn, "delete journal");
    conn.close();
    crate::fixtures::yydb::cleanup(&path);
}

#[test]
fn wal_reads_match_full_scan_across_reopen() {
    let path = crate::fixtures::yydb::temp_db_path("read-window-wal");
    let conn = Connection::open_with_flags(&path, OpenFlags::wal()).expect("open");
    conn.ensure_schema(SCHEMA).expect("schema");
    load(&conn, TOTAL);
    assert_parity(&conn, "wal before reopen");
    conn.close();
    drop(conn);

    let reopened = Connection::open_with_flags(&path, OpenFlags::wal()).expect("reopen");
    assert_parity(&reopened, "wal after reopen");
    reopened.close();
    crate::fixtures::yydb::cleanup(&path);
}

#[test]
fn in_memory_reads_match_full_scan() {
    let conn = Connection::open_in_memory().expect("open");
    conn.ensure_schema(SCHEMA).expect("schema");
    load(&conn, TOTAL);
    assert_parity(&conn, "in memory");
}

/// Object-backed payloads live outside the row keys. A narrowed read must still
/// hand the resolver a complete object catalog, otherwise the payload column
/// cannot be rebuilt.
#[test]
fn embedded_objects_resolve_inside_narrow_windows() {
    let schema = r#"
table Blob {
    @@id: utf8,
    payload: bytes,
}
"#;
    let path = crate::fixtures::yydb::temp_db_path("read-window-objects");
    let conn = Connection::open(&path).expect("open");
    conn.ensure_schema(schema).expect("schema");

    let large = INLINE_BYTES_MAX + 1_024;
    conn.begin().expect("begin");
    for index in 0..8_usize {
        let octets: Vec<u8> = (0..large).map(|offset| ((index * 7 + offset) % 251) as u8).collect();
        conn.execute_sql_write(
            "INSERT INTO Blob (id, payload) VALUES (?1, ?2)",
            &[Value::Text(format!("blob-{index:04}")), Value::Bytes(octets)],
        )
        .unwrap_or_else(|error| panic!("insert blob-{index}: {error}"));
    }
    conn.commit().expect("commit");

    let rows = rows_of(conn.query_sql("SELECT id, payload FROM Blob ORDER BY id").expect("scan"));
    for (index, (id, octets)) in rows.iter().enumerate() {
        assert_eq!(id, &format!("blob-{index:04}"));
        assert_eq!(octets.len(), large);
    }
    for index in [0_usize, 3, 7] {
        let expected: Vec<u8> = (0..large).map(|offset| ((index * 7 + offset) % 251) as u8).collect();
        let got = rows_of(
            conn.query_sql_with_params(
                "SELECT id, payload FROM Blob WHERE id = ?1",
                &[Value::Text(format!("blob-{index:04}"))],
            )
            .expect("point"),
        );
        assert_eq!(got, vec![(format!("blob-{index:04}"), expected)], "point blob-{index}");
    }
    conn.close();
    crate::fixtures::yydb::cleanup(&path);
}
