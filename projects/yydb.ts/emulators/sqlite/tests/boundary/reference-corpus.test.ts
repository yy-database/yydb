import assert from "node:assert/strict";
import test from "node:test";

import { SqliteDatabase, SqliteSnapshot, decodeRecord } from "@yydb/sqlite";
import {
    disposeReferenceFixture,
    generateReferenceFixture,
    matchesReference,
    referencePython,
} from "@yydb/sqlite-fixtures";

const SKIP_NO_PYTHON = referencePython() ? false : "no python3/python with sqlite3 on this host";

test("reference corpus empty table yields no rows", { timeout: 60_000, skip: SKIP_NO_PYTHON }, () => {
    const fixture = generateReferenceFixture(4096);
    assert.ok(fixture);
    try {
        const database = SqliteDatabase.open(fixture.path);
        assert.deepEqual(database.table("empty_table", { maxPages: 64, maxRows: 32 }), []);
        assert.deepEqual(new SqliteSnapshot(new Uint8Array(fixture.bytes)).tableDecoded("empty_table", { maxPages: 64, maxRows: 32 }), []);
    } finally {
        disposeReferenceFixture(fixture);
    }
});

test("reference corpus bulk scan enforces row limits", { timeout: 60_000, skip: SKIP_NO_PYTHON }, () => {
    const fixture = generateReferenceFixture(4096);
    assert.ok(fixture);
    try {
        const database = SqliteDatabase.open(fixture.path);
        assert.throws(
            () => database.table("bulk", { maxPages: 256, maxRows: 32 }),
            /row limit/,
        );
        const limited = database.table("bulk", { maxPages: 256, maxRows: 600 });
        assert.equal(limited.length, 600);
    } finally {
        disposeReferenceFixture(fixture);
    }
});

test("reference corpus overflow payloads honor per-row payload limits", { timeout: 60_000, skip: SKIP_NO_PYTHON }, () => {
    const fixture = generateReferenceFixture(4096);
    assert.ok(fixture);
    try {
        const database = SqliteDatabase.open(fixture.path);
        assert.throws(
            () => database.table("types", { maxPages: 64, maxRows: 8, maxPayloadBytes: 100 }),
            /payload limit/,
        );

        const rows = database.tableRaw("types", { maxPages: 64, maxRows: 8, maxPayloadBytes: 1_048_576 });
        const decoded = decodeRecord(rows[1].payload);
        const reference = fixture.manifest.tables.find((table) => table.name === "types")!.rows[1];
        for (const [index, column] of decoded.entries()) {
            assert.ok(
                matchesReference(column, reference.values[index]),
                `types overflow row column ${index}`,
            );
        }
    } finally {
        disposeReferenceFixture(fixture);
    }
});

test("sqlite_schema lists the same objects as sqlite_master", { timeout: 60_000, skip: SKIP_NO_PYTHON }, () => {
    const fixture = generateReferenceFixture(4096);
    assert.ok(fixture);
    try {
        const database = SqliteDatabase.open(fixture.path);
        const master = database.schema({ maxPages: 256 });
        const alias = database.tableRaw("sqlite_schema", { maxPages: 256, maxRows: 64 });
        assert.equal(alias.length, master.length);
    } finally {
        disposeReferenceFixture(fixture);
    }
});
