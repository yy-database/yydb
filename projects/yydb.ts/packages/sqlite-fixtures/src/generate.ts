import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { parseManifest, type ReferenceFixture } from "./manifest.ts";
import { referencePython } from "./python.ts";

const PYTHON_FIXTURE = `
import os
import sqlite3
import struct
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
CREATE TABLE "affinity" (
  id INTEGER PRIMARY KEY,
  v REAL
);
CREATE TABLE "bulk" (
  label TEXT,
  n INTEGER
);
"""

def hex_text(value):
    return value.encode("utf-8").hex()

def enc_sql(value):
    return "-" if value is None else "s" + hex_text(value)

def enc_default(value):
    return "-" if value is None else "d" + hex_text(value)

def enc_value(value):
    if value is None:
        return "n"
    if isinstance(value, int):
        return "i%d" % value
    if isinstance(value, float):
        return "f" + struct.pack(">d", value).hex()
    if isinstance(value, str):
        return "t" + value.encode("utf-8").hex()
    if isinstance(value, bytes):
        return "b" + value.hex()
    raise TypeError("unexpected SQLite value type: %r" % type(value))

db = sqlite3.connect(path)
try:
    db.execute("PRAGMA page_size=%d" % page_size)
    db.execute("PRAGMA journal_mode=DELETE")
    db.execute("PRAGMA auto_vacuum=NONE")
    db.executescript(DDL)
    db.executemany(
        'INSERT INTO "simple" (id, name, note) VALUES (?, ?, ?)',
        [(1, "alpha", None), (2, "00042", ""), (3, "中文", "note")],
    )
    db.executemany(
        'INSERT INTO "implicit" (rowid, a, b, c) VALUES (?, ?, ?, ?)',
        [
            (10, None, 7, 1.5),
            (2, "", None, -0.5),
            (5, "00001", -9223372036854775808, 1e300),
        ],
    )
    db.executemany(
        'INSERT INTO "types" (id, txt, lead, r, b, n, empty) VALUES (?, ?, ?, ?, ?, ?, ?)',
        [
            (1, "00042", "007", -0.5, b"", None, ""),
            (2, None, "", 0.25, bytes([0, 255]) * 3000, -9223372036854775808, None),
            # A durable YYDB row cell holds only a small inline payload, so the
            # second blob stays the small inline case while the 6000-byte one
            # above exercises the object-reference placement.
            (3, "line\\nbreak\\tend", "000", 1e300, bytes([0]) * 96, 9223372036854775807, ""),
        ],
    )
    db.executemany(
        'INSERT INTO "multi" (code, region, amount, note) VALUES (?, ?, ?, ?)',
        [("A", "east", 0.5, None), ("A", "west", 12.5, ""), ("B", "east", None, "x")],
    )
    db.executemany('INSERT INTO "wo" (k, v) VALUES (?, ?)', [("k1", 1), ("k2", None)])
    db.execute('INSERT INTO "affinity" (id, v) VALUES (1, 0.0)')
    db.executemany(
        'INSERT INTO "bulk" (rowid, label, n) VALUES (?, ?, ?)',
        [(rowid, "row-%03d" % rowid, rowid * 2) for rowid in range(600, 0, -1)],
    )
    db.commit()
    assert db.execute("PRAGMA page_size").fetchone()[0] == page_size, "page size not applied"
    assert db.execute("PRAGMA integrity_check").fetchone() == ("ok",), "fixture integrity check failed"
    lines = ["page\\t%d" % page_size, "encoding\\t" + db.execute("PRAGMA encoding").fetchone()[0].lower().replace("-", "")]
    for kind, name, table_name, root_page, sql in db.execute(
        "SELECT type, name, tbl_name, rootpage, sql FROM sqlite_master ORDER BY rowid"
    ):
        lines.append("\\t".join([
            "schema", kind, hex_text(name), hex_text(table_name), str(root_page), enc_sql(sql)
        ]))
    for table in ("simple", "implicit", "empty_table", "types", "multi", "wo", "affinity", "bulk"):
        for cid, name, declared, not_null, default, pk in db.execute('PRAGMA table_info("%s")' % table):
            lines.append("\\t".join([
                "column", hex_text(table), str(cid), hex_text(name), hex_text(declared),
                str(not_null), enc_default(default), str(pk),
            ]))
    for table in ("simple", "implicit", "empty_table", "types", "multi", "affinity", "bulk"):
        info = list(db.execute('PRAGMA table_info("%s")' % table))
        aliases = [row[0] for row in info if row[5] and row[2].strip().upper() == "INTEGER"]
        alias = aliases[0] if len(aliases) == 1 else None
        rows = list(db.execute('SELECT rowid, * FROM "%s" ORDER BY rowid' % table))
        lines.append("\\t".join(["rows", hex_text(table), str(len(rows))]))
        for row in rows:
            values = [enc_value(value) for value in row[1:]]
            if alias is not None:
                values[alias] = "n"
            lines.append("\\t".join(["row", str(row[0])] + values))
    with open(os.path.join(directory, "manifest.txt"), "w", encoding="utf-8", newline="\\n") as handle:
        handle.write("\\n".join(lines) + "\\n")
finally:
    db.close()
`;

export interface GeneratedFixture extends ReferenceFixture {
    directory: string;
    path: string;
}

/** Generates one upstream SQLite corpus fixture, or returns undefined when Python is unavailable. */
export function generateReferenceFixture(pageSize: number): GeneratedFixture | undefined {
    const python = referencePython();
    if (!python) {
        return undefined;
    }

    const directory = mkdtempSync(join(tmpdir(), `yydb-ref-corpus-${pageSize}-`));
    const output = spawnSync(python, ["-c", PYTHON_FIXTURE, directory, String(pageSize)], {
        timeout: 30_000,
        encoding: "utf8",
    });
    if (output.error || output.status !== 0) {
        rmSync(directory, { recursive: true, force: true });
        throw new Error(`reference SQLite fixture generation failed: ${output.stderr || output.stdout}`);
    }

    const path = join(directory, "db.sqlite");
    const bytes = readFileSync(path);
    const manifest = parseManifest(readFileSync(join(directory, "manifest.txt"), "utf8"));
    return {
        pageSize,
        bytes,
        manifest,
        directory,
        path,
    };
}

export function disposeReferenceFixture(fixture: GeneratedFixture): void {
    rmSync(fixture.directory, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 });
}
