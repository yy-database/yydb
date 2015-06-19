import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { Database } from "@yydb/yydb";
import {
    disposeReferenceFixture,
    generateReferenceFixture,
    referencePython,
} from "@yydb/sqlite-fixtures";
import {
    buildSimpleAdoptionPrograms,
    buildTypesAdoptionPrograms,
    SIMPLE_SCHEMA,
    TYPES_SCHEMA,
} from "@yydb/yydb-fixtures";

const SKIP_NO_PYTHON = referencePython() ? false : "no python3/python with sqlite3 on this host";

test("reference corpus simple table is adopted into @yydb/yydb", { timeout: 60_000, skip: SKIP_NO_PYTHON }, async () => {
    const fixture = generateReferenceFixture(4096);
    assert.ok(fixture);
    const directory = mkdtempSync(join(tmpdir(), "yydb-adopt-simple-"));
    try {
        const database = await Database.open(join(directory, "adopted.yydb"));
        await database.ensureSchema(SIMPLE_SCHEMA);
        for (const program of buildSimpleAdoptionPrograms(fixture.path)) {
            await database.execute(program.source, program.params);
        }
        const rows = (await database.query("Simple.all().collect()") as Array<{ id: number; name: string; note: string }>)
            .sort((left, right) => left.id - right.id);
        assert.equal(rows.length, 3);
        assert.equal(rows[0].name, "alpha");
        assert.equal(rows[1].name, "00042");
        assert.equal(rows[2].name, "中文");
        assert.equal(rows[2].note, "note");
        await database.close();
    } finally {
        disposeReferenceFixture(fixture);
        rmSync(directory, { recursive: true, force: true });
    }
});

test("reference corpus types table is adopted with blobs and edge integers", { timeout: 60_000, skip: SKIP_NO_PYTHON }, async () => {
    const fixture = generateReferenceFixture(4096);
    assert.ok(fixture);
    const directory = mkdtempSync(join(tmpdir(), "yydb-adopt-types-"));
    try {
        // The reference blob exceeds `INLINE_BYTES_MAX`, so this corpus needs the
        // `.yydx` layout with its sibling object root instead of a single file.
        const database = await Database.open(join(directory, "types.yydx"));
        await database.ensureSchema(TYPES_SCHEMA);
        for (const program of buildTypesAdoptionPrograms(fixture.path)) {
            await database.execute(program.source, program.params);
        }
        const rows = await database.query("TypesRow.all().collect()") as Array<{
            id: number;
            txt: string;
            n: number;
            payload: Uint8Array;
        }>;
        assert.equal(rows.length, 3);
        assert.equal(rows[0].txt, "00042");
        assert.equal(rows[1].n, -9223372036854775808);
        assert.ok(rows[1].payload instanceof Uint8Array, "the blob column must decode to octets");
        assert.ok(rows[1].payload.length > 5000, "the adopted blob must keep its length");
        await database.close();
    } finally {
        disposeReferenceFixture(fixture);
        rmSync(directory, { recursive: true, force: true });
    }
});
