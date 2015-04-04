import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { Database } from "@yydb/yydb";

import { buildCaseAdoptionProgram, CASE_ROW_SCHEMA, createCaseSqliteFixture } from "@yydb/yydb-fixtures";

test("SQLite rows are adopted into @yydb/yydb through VOS conversion", async () => {
    const sqlite = createCaseSqliteFixture();
    const directory = mkdtempSync(join(tmpdir(), "yydb-adopt-node-"));
    const yydbPath = join(directory, "adopted.yydb");
    try {
        const database = await Database.open(yydbPath);
        await database.ensureSchema(CASE_ROW_SCHEMA);

        for (const program of buildCaseAdoptionProgram(sqlite.path)) {
            await database.execute(program);
        }

        const rows = (await database.query("CaseRow.all().collect()") as Array<{ id: number; label: string }>)
            .sort((left, right) => left.id - right.id);
        assert.deepEqual(rows, [
            { id: 1, label: "第一" },
            { id: 2, label: "第二" },
        ]);
        await database.close();
    } finally {
        rmSync(sqlite.directory, { recursive: true, force: true });
        rmSync(directory, { recursive: true, force: true });
    }
});

test("@yydb/yydb does not read SQLite files directly; adoption is explicit", async () => {
    const sqlite = createCaseSqliteFixture([{ rowid: 1, label: "只应经转换" }]);
    const directory = mkdtempSync(join(tmpdir(), "yydb-adopt-explicit-"));
    const yydbPath = join(directory, "empty.yydb");
    try {
        const database = await Database.open(yydbPath);
        await database.ensureSchema(CASE_ROW_SCHEMA);

        const before = await database.query("CaseRow.all().collect()");
        assert.deepEqual(before, [], "opening YYDB must not auto-import SQLite bytes");

        for (const program of buildCaseAdoptionProgram(sqlite.path)) {
            await database.execute(program);
        }

        const after = await database.query("CaseRow.all().collect()");
        assert.deepEqual(after, [{ id: 1, label: "只应经转换" }]);
        await database.close();
    } finally {
        rmSync(sqlite.directory, { recursive: true, force: true });
        rmSync(directory, { recursive: true, force: true });
    }
});
