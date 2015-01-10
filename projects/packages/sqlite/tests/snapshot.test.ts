import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { isYydsNativeInstalled } from "@yydb/yyds/node";
import { SqliteSnapshot } from "../src/node.ts";

const nativeAvailable = isYydsNativeInstalled();
if (process.env.YYDS_SQLITE_NATIVE_REQUIRED === "1") {
    assert.ok(nativeAvailable, "native SQLite tests require the built platform addon");
}

test("snapshot constructor rejects invalid inputs before loading an addon", () => {
    for (const limit of [-1, 0.5, NaN, Infinity, 0x1_0000_0000]) {
        assert.throws(() => new SqliteSnapshot(new Uint8Array(), limit), /unsigned 32-bit/);
    }
    assert.throws(() => new SqliteSnapshot(Buffer.alloc(2), 1), /byte limit/);
    assert.throws(() => new SqliteSnapshot("bytes" as unknown as Uint8Array), /Uint8Array/);
});

test("native snapshot copies bytes and exposes bounded schema scans", {
    skip: !nativeAvailable,
}, () => {
    const bytes = readFileSync(
        new URL("../../../crates/sqlite/src/assets/blank.sqlite", import.meta.url),
    );
    const snapshot = new SqliteSnapshot(bytes);
    bytes.fill(0);
    assert.deepEqual(snapshot.schema(), []);
    assert.deepEqual(snapshot.table("sqlite_master"), []);
    assert.throws(() => snapshot.schema({ maxPages: 0 }), /page limit/);
    assert.throws(() => snapshot.table("missing"), /not found/);
    assert.throws(() => new SqliteSnapshot(bytes), /magic/);
    assert.deepEqual(new SqliteSnapshot(Buffer.alloc(0), 0).schema(), []);
    for (const limit of [-1, 0.5, NaN, Infinity, 0x1_0000_0000]) {
        assert.throws(() => snapshot.schema({ maxRows: limit }), /unsigned 32-bit/);
    }
    assert.throws(() => snapshot.schema({ unexpected: 1 } as never), /Unknown SQLite/);
    assert.throws(() => snapshot.schema(null as never), /must be an object/);
    assert.throws(() => snapshot.table(42 as never), /must be a string/);
});

test("Node native reader interoperates with original SQLite files", {
    skip: !nativeAvailable || !process.env.YYDS_SQLITE_REFERENCE_PYTHON,
}, () => {
    const directory = mkdtempSync(join(tmpdir(), "yyds-sqlite-node-"));
    const path = join(directory, "reference.sqlite");
    const sql = 'CREATE TABLE "案例" (id INTEGER PRIMARY KEY, payload BLOB)';
    const script = `
import sqlite3, sys
with sqlite3.connect(sys.argv[1]) as db:
    db.execute("pragma encoding='UTF-16le'")
    db.execute(sys.argv[2])
    db.executemany('insert into "案例" values (?, ?)', [
        (-9223372036854775808, b'\\x00\\xff' * 5000),
        (9223372036854775807, b'\\xff\\x00' * 5000),
    ])
    db.execute('create index payload_index on "案例"(payload)')
    db.execute('create table unique_probe (value text unique)')
    assert db.execute('pragma integrity_check').fetchone() == ('ok',)
`;
    const output = spawnSync(process.env.YYDS_SQLITE_REFERENCE_PYTHON!, ["-c", script, path, sql], {
        timeout: 15_000,
        encoding: "utf8",
    });
    assert.ifError(output.error);
    assert.equal(output.status, 0, output.stderr);
    const original = readFileSync(path);
    const snapshot = new SqliteSnapshot(original);
    const schema = snapshot.schema();
    const table = schema.find((object) => object.name === "案例");
    assert.equal(table?.kind, "table");
    assert.equal(table?.tableName, "案例");
    assert.equal(table?.sql, sql);
    assert.equal(typeof table?.rootPage, "number");
    const automatic = schema.find((object) => object.name === "sqlite_autoindex_unique_probe_1");
    assert.equal(automatic?.kind, "index");
    assert.equal(automatic?.sql, undefined);
    const rows = snapshot.table("案例");
    assert.deepEqual(
        rows.map((row) => row.rowid),
        [-(1n << 63n), (1n << 63n) - 1n],
    );
    for (const [index, row] of rows.entries()) {
        assert.ok(Buffer.isBuffer(row.payload));
        assert.ok(row.payload.length > 10_000);
        const expected = Buffer.alloc(10_000);
        for (let offset = 0; offset < expected.length; offset += 2) {
            expected[offset] = index === 0 ? 0 : 255;
            expected[offset + 1] = index === 0 ? 255 : 0;
        }
        assert.deepEqual(row.payload.subarray(-10_000), expected);
    }
    assert.throws(() => snapshot.table("案例", { maxRows: 1 }), /row limit/);
    assert.throws(() => snapshot.table("案例", { maxPayloadBytes: 100 }), /payload limit/);
    assert.deepEqual(readFileSync(path), original);
});
