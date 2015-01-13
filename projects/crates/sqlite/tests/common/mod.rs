//! Fixture generation and manifest parsing for real SQLite interoperability tests.
//!
//! Every fixture is produced inside the test at runtime by the upstream SQLite
//! engine (Python's standard `sqlite3` module) and deleted when the test ends.
//! No binary database files are checked into the repository.

use std::{
    env, fs,
    path::PathBuf,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

/// Upstream SQLite fixture builder. Writes `db.sqlite` and `manifest.txt`.
const PYTHON_FIXTURE: &str = r#"
import os
import sqlite3
import sys

directory = sys.argv[1]
page_size = int(sys.argv[2])
path = os.path.join(directory, "db.sqlite")
if os.path.exists(path):
    os.remove(path)

DDL = """
CREATE TABLE "simple" (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL DEFAULT 'unknown',
  note TEXT
);
CREATE TABLE "implicit" (
  a TEXT,
  b INTEGER,
  c REAL
);
CREATE TABLE "empty_table" (
  id INTEGER PRIMARY KEY,
  v TEXT
);
CREATE TABLE "types" (
  id INTEGER PRIMARY KEY,
  txt TEXT,
  lead TEXT,
  r REAL,
  b BLOB,
  n INTEGER,
  empty TEXT
);
CREATE TABLE "multi" (
  code TEXT NOT NULL,
  region TEXT NOT NULL,
  amount REAL DEFAULT 0,
  note TEXT,
  PRIMARY KEY (code, region)
);
CREATE INDEX "multi_amount_idx" ON "multi" (amount, region, code);
CREATE TABLE "wo" (
  k TEXT PRIMARY KEY,
  v INTEGER
) WITHOUT ROWID;
CREATE VIEW "v_simple" AS SELECT id, name FROM "simple";
"""

def hex_text(value):
    return value.encode("utf-8").hex()

def enc_sql(value):
    return "-" if value is None else "s" + hex_text(value)

def enc_default(value):
    return "-" if value is None else "d" + hex_text(value)

db = sqlite3.connect(path)
try:
    db.execute("PRAGMA page_size=%d" % page_size)
    db.execute("PRAGMA journal_mode=DELETE")
    db.execute("PRAGMA auto_vacuum=NONE")
    db.executescript(DDL)
    db.commit()
    assert db.execute("PRAGMA page_size").fetchone()[0] == page_size, "page size not applied"
    assert db.execute("PRAGMA integrity_check").fetchone() == ("ok",), "fixture integrity check failed"
    lines = ["page\t%d" % page_size, "encoding\t" + db.execute("PRAGMA encoding").fetchone()[0].lower().replace("-", "")]
    for kind, name, table_name, root_page, sql in db.execute(
        "SELECT type, name, tbl_name, rootpage, sql FROM sqlite_master ORDER BY rowid"
    ):
        lines.append("\t".join([
            "schema", kind, hex_text(name), hex_text(table_name), str(root_page), enc_sql(sql)
        ]))
    for table in ("simple", "implicit", "empty_table", "types", "multi", "wo"):
        for cid, name, declared, not_null, default, pk in db.execute('PRAGMA table_info("%s")' % table):
            lines.append("\t".join([
                "column", hex_text(table), str(cid), hex_text(name), hex_text(declared),
                str(not_null), enc_default(default), str(pk),
            ]))
    with open(os.path.join(directory, "manifest.txt"), "w", encoding="utf-8") as handle:
        handle.write("\n".join(lines) + "\n")
finally:
    db.close()
"#;

/// One schema row as reported by `sqlite_master`.
#[derive(Debug)]
pub struct ReferenceSchema {
    pub kind: String,
    pub name: String,
    pub table_name: String,
    pub root_page: i64,
    pub sql: Option<String>,
}

/// One `PRAGMA table_info` row: cid, name, declared type, NOT NULL, default, PK order.
#[derive(Debug)]
pub struct ReferenceColumn {
    pub table: String,
    pub cid: i64,
    pub name: String,
    pub declared_type: String,
    pub not_null: bool,
    pub default: Option<String>,
    pub primary_key: i64,
}

/// Parsed fixture manifest produced by upstream SQLite.
#[derive(Debug)]
pub struct Manifest {
    pub page_size: usize,
    pub encoding: String,
    pub schema: Vec<ReferenceSchema>,
    pub columns: Vec<ReferenceColumn>,
}

/// Generated fixture plus the upstream-SQLite manifest describing it.
pub struct Fixture {
    pub page_size: usize,
    pub bytes: Vec<u8>,
    pub manifest: Manifest,
    directory: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

type ExpectedColumn = (i64, &'static str, &'static str, bool, Option<&'static str>, i64);

/// Declared schema that the generated fixture must expose through `PRAGMA table_info`.
/// Comparing these rows with the upstream manifest and the reader's preserved CREATE
/// source is how column names, types, NOT NULL, defaults, and key order are checked.
pub const EXPECTED_COLUMNS: &[(&str, &[ExpectedColumn])] = &[
    (
        "simple",
        &[
            (0, "id", "INTEGER", false, None, 1),
            (1, "name", "TEXT", true, Some("'unknown'"), 0),
            (2, "note", "TEXT", false, None, 0),
        ],
    ),
    ("implicit", &[(0, "a", "TEXT", false, None, 0), (1, "b", "INTEGER", false, None, 0), (2, "c", "REAL", false, None, 0)]),
    ("empty_table", &[(0, "id", "INTEGER", false, None, 1), (1, "v", "TEXT", false, None, 0)]),
    (
        "types",
        &[
            (0, "id", "INTEGER", false, None, 1),
            (1, "txt", "TEXT", false, None, 0),
            (2, "lead", "TEXT", false, None, 0),
            (3, "r", "REAL", false, None, 0),
            (4, "b", "BLOB", false, None, 0),
            (5, "n", "INTEGER", false, None, 0),
            (6, "empty", "TEXT", false, None, 0),
        ],
    ),
    (
        "multi",
        &[
            (0, "code", "TEXT", true, None, 1),
            (1, "region", "TEXT", true, None, 2),
            (2, "amount", "REAL", false, Some("0"), 0),
            (3, "note", "TEXT", false, None, 0),
        ],
    ),
    ("wo", &[(0, "k", "TEXT", true, None, 1), (1, "v", "INTEGER", false, None, 0)]),
];

/// Generates one fixture with the requested page size, or returns `None` when no
/// Python interpreter with the `sqlite3` module is available on this host.
pub fn generate(page_size: usize) -> Option<Fixture> {
    let python = reference_python()?;
    let directory = env::temp_dir().join(format!(
        "yydb-sqlite-interop-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).expect("clock before epoch").as_nanos()
    ));
    fs::create_dir_all(&directory).expect("create fixture directory");

    let output = Command::new(python)
        .arg("-c")
        .arg(PYTHON_FIXTURE)
        .arg(&directory)
        .arg(page_size.to_string())
        .output()
        .expect("spawn reference python");
    assert!(output.status.success(), "reference SQLite fixture generation failed: {}", String::from_utf8_lossy(&output.stderr));

    let bytes = fs::read(directory.join("db.sqlite")).expect("read generated fixture");
    let manifest = parse_manifest(&fs::read_to_string(directory.join("manifest.txt")).expect("read fixture manifest"));
    assert_eq!(manifest.page_size, page_size);
    Some(Fixture { page_size, bytes, manifest, directory })
}

fn reference_python() -> Option<String> {
    let candidates = match env::var("YYDS_SQLITE_REFERENCE_PYTHON") {
        Ok(configured) if !configured.is_empty() => vec![configured],
        _ => vec!["python3".to_owned(), "python".to_owned()],
    };
    candidates.into_iter().find(|candidate| {
        Command::new(candidate)
            .args(["-c", "import sqlite3"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

fn parse_manifest(text: &str) -> Manifest {
    let mut manifest = Manifest { page_size: 0, encoding: String::new(), schema: Vec::new(), columns: Vec::new() };
    for line in text.lines().filter(|line| !line.is_empty()) {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields[0] {
            "page" => manifest.page_size = fields[1].parse().expect("manifest page size"),
            "encoding" => manifest.encoding = fields[1].to_owned(),
            "schema" => {
                assert_eq!(fields.len(), 6, "malformed schema record");
                manifest.schema.push(ReferenceSchema {
                    kind: fields[1].to_owned(),
                    name: hex_text(fields[2]),
                    table_name: hex_text(fields[3]),
                    root_page: fields[4].parse().expect("manifest root page"),
                    sql: optional_text(fields[5], 's'),
                });
            }
            "column" => {
                assert_eq!(fields.len(), 8, "malformed column record");
                manifest.columns.push(ReferenceColumn {
                    table: hex_text(fields[1]),
                    cid: fields[2].parse().expect("manifest column id"),
                    name: hex_text(fields[3]),
                    declared_type: hex_text(fields[4]),
                    not_null: fields[5] != "0",
                    default: optional_text(fields[6], 'd'),
                    primary_key: fields[7].parse().expect("manifest primary key"),
                });
            }
            other => panic!("unexpected fixture manifest record {other}"),
        }
    }
    manifest
}

fn optional_text(field: &str, prefix: char) -> Option<String> {
    if field == "-" {
        return None;
    }
    Some(hex_text(field.strip_prefix(prefix).expect("manifest text prefix")))
}

fn hex_text(field: &str) -> String {
    assert!(field.len().is_multiple_of(2), "odd hex field");
    let bytes: Vec<u8> = (0..field.len() / 2)
        .map(|index| u8::from_str_radix(&field[index * 2..index * 2 + 2], 16).expect("hex manifest field"))
        .collect();
    String::from_utf8(bytes).expect("manifest text is UTF-8")
}

/// Header-claimed text encoding label, mirroring the reader's public names.
pub fn header_encoding(bytes: &[u8]) -> &'static str {
    match u32::from_be_bytes(bytes[56..60].try_into().unwrap()) {
        1 => "utf8",
        2 => "utf16le",
        3 => "utf16be",
        other => panic!("unexpected text encoding {other}"),
    }
}
