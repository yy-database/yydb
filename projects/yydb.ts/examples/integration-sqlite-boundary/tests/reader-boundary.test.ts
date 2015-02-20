import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import * as sqlite from "@yydb/sqlite";
import { SqliteDatabase, SqliteSnapshot } from "@yydb/sqlite";

import { createLimitationFixture } from "./fixtures/node-limitations.ts";
import { createNodeSamplesFixture } from "./fixtures/node-samples.ts";
import { READ_LIMITS } from "./fixtures/shared.ts";
import { YY_STORAGE_EXTENSIONS, writeYyStorageStub } from "./fixtures/yy-storage-stubs.ts";

const SQLITE_READER_EXPORTS = [
    "SqliteSnapshot",
    "SqliteDatabase",
    "decodeRecord",
] as const;

const YYDB_ENGINE_EXPORTS = [
    "Database",
    "ensureSchema",
    "query",
    "execute",
    "prepare",
    "initWasm",
    "loadYydsNative",
] as const;

test("@yydb/sqlite exposes only the read-only snapshot reader surface", () => {
    const exportNames = Object.keys(sqlite).sort();
    assert.deepEqual(exportNames, [...SQLITE_READER_EXPORTS].sort());

    for (const forbidden of YYDB_ENGINE_EXPORTS) {
        assert.equal(forbidden in sqlite, false, `@yydb/sqlite must not export ${forbidden}`);
    }

    const proto = SqliteDatabase.prototype as unknown as Record<string, unknown>;
    for (const method of ["ensureSchema", "query", "execute", "prepare", "put", "get", "batch"]) {
        assert.equal(method in proto, false, `SqliteDatabase must not implement ${method}`);
    }
});

test("@yydb/sqlite rejects YYDB, YYDS, YYKV, and YYDX storage bytes", () => {
    for (const extension of YY_STORAGE_EXTENSIONS) {
        const fixture = writeYyStorageStub(extension);
        try {
            assert.throws(
                () => SqliteDatabase.open(fixture.path),
                /magic/i,
                `SqliteDatabase.open must reject ${extension}`,
            );
            assert.throws(
                () => new SqliteSnapshot(new Uint8Array(fixture.bytes)),
                /magic/i,
                `SqliteSnapshot must reject ${extension}`,
            );
        } finally {
            rmSync(fixture.directory, { recursive: true, force: true });
        }
    }
});

test("@yydb/sqlite does not execute VOS or SQL against a populated .yydb file", async () => {
    const { Database } = await import("@yydb/yydb");
    const directory = mkdtempSync(join(tmpdir(), "yydb-sqlite-vos-negative-"));
    const yydbPath = join(directory, "probe.yydb");
    try {
        const engine = await Database.open(yydbPath);
        await engine.ensureSchema(`table Setting {
            @@id: uuid,
            key: utf8,
            value: utf8,
        }`);
        await engine.put("theme", "dark");
        assert.equal(new TextDecoder().decode(await engine.get("theme")!), "dark");
        await engine.close();

        const reader = SqliteDatabase.openInMemory() as unknown as Record<string, unknown>;
        assert.equal(typeof reader.ensureSchema, "undefined");
        assert.equal(typeof reader.query, "undefined");
        assert.equal(typeof reader.execute, "undefined");

        assert.throws(
            () => SqliteDatabase.open(yydbPath),
            /magic mismatch/i,
            "a populated .yydb must not be readable as SQLite format-3",
        );
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("@yydb/sqlite rejects SQLite bytes even when the path ends in .yydb", () => {
    const fixture = createNodeSamplesFixture();
    const directory = mkdtempSync(join(tmpdir(), "yydb-sqlite-extension-"));
    const disguised = join(directory, "disguised.yydb");
    try {
        writeFileSync(disguised, readFileSync(fixture.path));
        const database = SqliteDatabase.open(disguised);
        assert.equal(database.table("samples", READ_LIMITS).length, 2, "magic wins over extension");
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
        rmSync(directory, { recursive: true, force: true });
    }
});

test("@yydb/sqlite rejects invalid snapshot inputs and scan limits", () => {
    const fixture = createNodeSamplesFixture();
    try {
        for (const limit of [-1, 0.5, NaN, Infinity, 0x1_0000_0000]) {
            assert.throws(() => new SqliteSnapshot(new Uint8Array(), limit), /unsigned 32-bit/);
        }
        assert.throws(() => new SqliteSnapshot("bytes" as unknown as Uint8Array), /Uint8Array/);
        assert.throws(() => new SqliteSnapshot(Buffer.alloc(2), 1), /byte limit/);

        const database = SqliteDatabase.open(fixture.path);
        assert.throws(() => database.schema({ maxPages: 0 }), /page limit/);
        assert.throws(() => database.schema({ unexpected: 1 } as never), /Unknown SQLite/);
        assert.throws(() => database.schema(null as never), /must be an object/);
        assert.throws(() => database.table(42 as never), /must be a string/);
        assert.throws(() => database.table("missing"), /not found/);
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});

test("@yydb/sqlite reports explicit errors for views and WITHOUT ROWID tables", () => {
    const fixture = createLimitationFixture();
    try {
        const database = SqliteDatabase.open(fixture.path);

        assert.throws(() => database.table("wo", READ_LIMITS), /index b-tree scan/i);
        assert.throws(() => database.table("v_rowid", READ_LIMITS), /view execution/i);

        const schema = database.schema(READ_LIMITS);
        assert.ok(schema.some((object) => object.kind === "view" && object.name === "v_rowid"));
        assert.ok(schema.some((object) => object.kind === "table" && object.name === "wo"));
        assert.equal(database.table("rowid_table", READ_LIMITS).length, 1);
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});

test("@yydb/sqlite enforces row limits and leaves the source file unchanged", () => {
    const fixture = createNodeSamplesFixture();
    try {
        const database = SqliteDatabase.open(fixture.path);
        const before = fixture.bytes;

        assert.throws(() => database.table("samples", { maxRows: 1 }), /row limit/);
        assert.deepEqual(fixture.bytes, before, "read-only scans must not mutate the backing file");
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});
