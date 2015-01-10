import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { isYydsNativeInstalled } from "@yydb/yyds/node";

const cli = fileURLToPath(new URL("../bin/sqlite3.mjs", import.meta.url));

function run(...args: string[]) {
    const output = spawnSync(process.execPath, ["--experimental-strip-types", cli, ...args], {
        timeout: 10_000,
        encoding: "utf8",
    });
    assert.ifError(output.error);
    return output;
}

test("SQLite CLI reports the snapshot reader and rejects unsupported input", {
    skip: !isYydsNativeInstalled(),
}, () => {
    const version = run("--version");
    assert.equal(version.status, 0, version.stderr);
    assert.match(version.stdout, /^sqlite3 3\.45\.0 \(YYDS pure-Rust format-3 reader\)/);
    assert.equal(run("missing.sqlite").status, 1);
    assert.equal(run("missing.sqlite", "SELECT 1").status, 1);
    assert.equal(run("missing.sqlite", ".not-supported").status, 1);
    assert.equal(run("--unknown").status, 1);
});

test("SQLite CLI lists schema from a reference database file", {
    skip: !isYydsNativeInstalled() || !process.env.YYDS_SQLITE_REFERENCE_PYTHON,
}, () => {
    const directory = mkdtempSync(join(tmpdir(), "yyds-sqlite-cli-snapshot-"));
    const path = join(directory, "reference.sqlite");
    const sql = "CREATE TABLE samples (id INTEGER PRIMARY KEY, value TEXT, payload BLOB)";
    const script = `
import sqlite3, sys
with sqlite3.connect(sys.argv[1]) as db:
    db.execute(sys.argv[2])
    db.execute("INSERT INTO samples VALUES (1, 'hello', X'00ff')")
`;
    const created = spawnSync(process.env.YYDS_SQLITE_REFERENCE_PYTHON!, ["-c", script, path, sql], {
        timeout: 10_000,
        encoding: "utf8",
    });
    assert.ifError(created.error);
    assert.equal(created.status, 0, created.stderr);

    assert.equal(run(path, ".tables").stdout, "samples\n");
    assert.equal(run(path, ".schema").stdout, `${sql};\n`);
    assert.equal(readFileSync(path).subarray(0, 16).toString("binary"), "SQLite format 3\0");
});
