import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { Database } from "@yydb/yydb";

import { CASE_ROW_SCHEMA, createCaseSqliteFixture } from "@yydb/yydb-fixtures";

const SQL_REJECTIONS = [
    "SELECT 1",
    "INSERT INTO t VALUES (1)",
    "CREATE TABLE t (id INTEGER)",
    "PRAGMA schema_version",
    "SELECT a.* FROM simple a JOIN implicit b ON a.id = b.rowid",
    "DELETE FROM simple WHERE id = 1",
    "UPDATE simple SET name = 'x' WHERE id = 1",
] as const;

test("@yydb/yydb rejects a broad SQL surface area", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-sql-surface-"));
    try {
        const database = await Database.open(join(directory, "sql.yydb"));
        await database.ensureSchema(CASE_ROW_SCHEMA);

        for (const sql of SQL_REJECTIONS) {
            await assert.rejects(() => database.query(sql), /VOS|expected VOS expression/i, `query: ${sql}`);
            await assert.rejects(() => database.execute(sql), /VOS|expected VOS expression/i, `execute: ${sql}`);
        }

        await assert.rejects(
            () => database.prepare("SELECT id FROM CaseRow"),
            /VOS|expected VOS expression/i,
        );

        await database.close();
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("@yydb/yydb rejects YYDS and YYDX byte headers even when the path ends in .yydb", async () => {
    const stubs: Array<{ name: string; bytes: Buffer }> = [
        { name: "yyds.yydb", bytes: Buffer.concat([Buffer.from("YYDS\x01"), Buffer.alloc(128)]) },
        { name: "yydx.yydb", bytes: Buffer.concat([Buffer.from("YYDX\x01"), Buffer.alloc(128)]) },
    ];
    const directory = mkdtempSync(join(tmpdir(), "yydb-storage-stubs-"));
    try {
        for (const stub of stubs) {
            const path = join(directory, stub.name);
            writeFileSync(path, stub.bytes);
            await assert.rejects(() => Database.open(path), /corrupt|magic|YDPG/i, stub.name);
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("@yydb/yydb put/get remains independent from SQLite file paths", async () => {
    const sqlite = createCaseSqliteFixture();
    const directory = mkdtempSync(join(tmpdir(), "yydb-kv-isolation-"));
    const yydbPath = join(directory, "kv.yydb");
    try {
        const database = await Database.open(yydbPath);
        await database.ensureSchema(CASE_ROW_SCHEMA);
        await database.put("catalog/source", new TextEncoder().encode(sqlite.path));
        assert.equal(new TextDecoder().decode(await database.get("catalog/source")!), sqlite.path);
        assert.deepEqual(await database.query("CaseRow.all().collect()"), []);
        await database.close();
    } finally {
        rmSync(sqlite.directory, { recursive: true, force: true });
        rmSync(directory, { recursive: true, force: true });
    }
});

test("@yydb/yydb disguised SQLite bytes still fail format validation", async () => {
    const sqlite = createCaseSqliteFixture();
    const directory = mkdtempSync(join(tmpdir(), "yydb-disguised-sqlite-"));
    const disguised = join(directory, "looks-like-yydb.yydb");
    try {
        writeFileSync(disguised, readFileSync(sqlite.path));
        await assert.rejects(() => Database.open(disguised), /not YDPG|corrupt/i);
    } finally {
        rmSync(sqlite.directory, { recursive: true, force: true });
        rmSync(directory, { recursive: true, force: true });
    }
});
