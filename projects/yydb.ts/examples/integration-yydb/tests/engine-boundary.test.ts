import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { Database } from "@yydb/yydb";

import { CASE_ROW_SCHEMA, createCaseSqliteFixture } from "@yydb/yydb-fixtures";

const SQL_SAMPLES = [
    'SELECT id, label FROM "案例"',
    "INSERT INTO CaseRow (id, label) VALUES (1, '第一')",
    "CREATE TABLE CaseRow (id INTEGER PRIMARY KEY, label TEXT)",
] as const;

test("@yydb/yydb rejects SQL and only accepts VOS programs", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-engine-sql-negative-"));
    const path = join(directory, "engine.yydb");
    try {
        const database = await Database.open(path);
        await database.ensureSchema(CASE_ROW_SCHEMA);

        for (const sql of SQL_SAMPLES) {
            await assert.rejects(
                () => database.query(sql),
                /VOS|expected VOS expression/i,
                `query must reject SQL: ${sql}`,
            );
            await assert.rejects(
                () => database.execute(sql),
                /VOS|expected VOS expression/i,
                `execute must reject SQL: ${sql}`,
            );
        }

        await database.execute('CaseRow { id: 1, label: "via-vos" }.insert()');
        const rows = await database.query("CaseRow.all().collect()");
        assert.deepEqual(rows, [{ id: 1, label: "via-vos" }]);
        await database.close();
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("@yydb/yydb cannot open SQLite storage bytes as a YYDB file", async () => {
    const sqlite = createCaseSqliteFixture();
    const directory = mkdtempSync(join(tmpdir(), "yydb-engine-storage-negative-"));
    const disguised = join(directory, "case.yydb");
    try {
        writeFileSync(disguised, readFileSync(sqlite.path));
        await assert.rejects(
            () => Database.open(disguised),
            /not YDPG|corrupt/i,
            "SQLite bytes must not be accepted as YYDB storage",
        );
    } finally {
        rmSync(sqlite.directory, { recursive: true, force: true });
        rmSync(directory, { recursive: true, force: true });
    }
});

test("@yydb/yydb does not expose the SQLite snapshot reader", async () => {
    const yydb = await import("@yydb/yydb");
    assert.equal("SqliteDatabase" in yydb, false);
    assert.equal("SqliteSnapshot" in yydb, false);

    const proto = Database.prototype as unknown as Record<string, unknown>;
    for (const method of ["openSqlite", "importSqlite", "readSqlite", "table", "schema"]) {
        assert.equal(method in proto, false, `@yydb/yydb must not implement ${method}`);
    }
});
