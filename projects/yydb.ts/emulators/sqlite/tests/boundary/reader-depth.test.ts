import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { SqliteDatabase, SqliteSnapshot, decodeRecord } from "@yydb/sqlite";

import { createLimitationFixture } from "./fixtures/node-limitations.ts";
import { createNodeSamplesFixture } from "./fixtures/node-samples.ts";
import { READ_LIMITS } from "./fixtures/shared.ts";

test("@yydb/sqlite snapshot and file readers stay equivalent on samples", () => {
    const fixture = createNodeSamplesFixture();
    try {
        const file = SqliteDatabase.open(fixture.path);
        const snapshot = new SqliteSnapshot(new Uint8Array(fixture.bytes));

        assert.equal(file.sqliteVersion(), snapshot.sqliteVersion());
        assert.equal(file.encoding(), snapshot.encoding());
        assert.equal(file.ping(), snapshot.ping());
        assert.deepEqual(file.schema(READ_LIMITS), snapshot.schema(READ_LIMITS));
        assert.deepEqual(file.table("samples", READ_LIMITS), snapshot.tableDecoded("samples", READ_LIMITS));
        assert.deepEqual(file.tableRaw("samples", READ_LIMITS), snapshot.table("samples", READ_LIMITS));
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});

test("@yydb/sqlite rejects truncated and corrupt format-3 headers", () => {
    const fixture = createNodeSamplesFixture();
    const directory = mkdtempSync(join(tmpdir(), "yydb-sqlite-corrupt-"));
    try {
        const truncated = join(directory, "truncated.sqlite");
        writeFileSync(truncated, fixture.bytes.subarray(0, 512));
        assert.throws(() => SqliteDatabase.open(truncated), /corrupt|page|magic/i);

        const corrupt = join(directory, "corrupt.sqlite");
        const broken = Buffer.from(fixture.bytes);
        broken[0] = 0x58;
        writeFileSync(corrupt, broken);
        assert.throws(() => new SqliteSnapshot(new Uint8Array(broken)), /magic/i);
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
        rmSync(directory, { recursive: true, force: true });
    }
});

test("@yydb/sqlite reports virtual tables without evaluating them", () => {
    const fixture = createLimitationFixture();
    try {
        const database = SqliteDatabase.open(fixture.path);
        const schema = database.schema(READ_LIMITS);
        const virtualTable = schema.find((object) => object.name === "virt");
        assert.ok(virtualTable, "virtual table must appear in schema");
        assert.throws(() => database.table("virt", READ_LIMITS), /virtual table/i);
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});

test("@yydb/sqlite enforces total payload byte limits across a scan", () => {
    const fixture = createNodeSamplesFixture();
    try {
        const database = SqliteDatabase.open(fixture.path);
        assert.throws(
            () => database.table("samples", { maxPages: 64, maxRows: 32, maxTotalPayloadBytes: 1 }),
            /payload/i,
        );
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});

test("decodeRecord rejects invalid payloads", () => {
    const fixture = createNodeSamplesFixture();
    try {
        const row = SqliteDatabase.open(fixture.path).tableRaw("samples", READ_LIMITS)[0];
        assert.throws(() => decodeRecord(Buffer.from([0x00])), /corrupt|record|payload/i);
        assert.throws(() => decodeRecord(row.payload, "nope" as never), /encoding/i);
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});
