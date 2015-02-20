import assert from "node:assert/strict";
import { readFileSync, rmSync } from "node:fs";
import test from "node:test";

import { SqliteDatabase, SqliteSnapshot } from "@yydb/sqlite";

import {
    assertSamplesMatchYyds,
    createNodeCaseFixture,
    createNodeSamplesFixture,
    nodeInsertCase,
    nodeInsertSample,
    nodeUpdateSampleLabel,
} from "./fixtures/node-sqlite.ts";
import {
    columnText,
    MAGIC,
    READ_LIMITS,
    rowById,
    SAMPLES_DDL,
} from "./fixtures/shared.ts";

test("node:sqlite creates a samples database that @yydb/sqlite can read", { timeout: 20_000 }, () => {
    const fixture = createNodeSamplesFixture();
    try {
        const database = SqliteDatabase.open(fixture.path);
        assert.equal(database.path(), fixture.path);

        const schema = database.schema(READ_LIMITS);
        const samples = schema.find((object) => object.name === "samples");
        assert.ok(samples, "schema must contain the samples table");
        assert.equal(samples.kind, "table");
        assert.equal(samples.sql, SAMPLES_DDL);

        const rows = database.table("samples", READ_LIMITS);
        assert.equal(rows.length, 2);

        const first = rowById(rows, 1n);
        assert.equal(first.columns[0].kind, "null", "INTEGER PRIMARY KEY alias stores NULL in the record");
        assert.equal(first.columns[1].valueInteger, 42n);
        assert.equal(first.columns[2].valueReal, 3.5);
        assert.equal(first.columns[3].valueText, "alpha");
        assert.deepEqual(first.columns[4].valueBlob, Buffer.from([0x00, 0xff, 0x10]));
        assert.equal(first.columns[5].kind, "null");

        assert.deepEqual(database.pages().subarray(0, MAGIC.length), MAGIC);
        assert.equal(database.ping(), 1);
        assert.deepEqual(readFileSync(fixture.path), fixture.bytes, "reader must not mutate the file");
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});

test("node:sqlite creates a 案例 database that @yydb/sqlite can read", { timeout: 20_000 }, () => {
    const fixture = createNodeCaseFixture();
    try {
        const database = SqliteDatabase.open(fixture.path);
        const table = database.schema(READ_LIMITS).find((object) => object.name === "案例");
        assert.ok(table);
        assert.equal(table.tableName, "案例");

        const rows = database.table("案例", READ_LIMITS);
        assert.equal(rows.length, 2);
        assert.equal(columnText(rowById(rows, 1n).columns[1]), "第一");
        assert.equal(columnText(rowById(rows, 2n).columns[1]), "第二");
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});

test("node:sqlite modifications remain visible to @yydb/sqlite", { timeout: 20_000 }, () => {
    const fixture = createNodeSamplesFixture();
    try {
        nodeInsertSample(fixture.path, {
            rowid: 3,
            quantity: 99,
            score: 1.25,
            label: "gamma",
            payload: Buffer.from([0xab]),
            note: "added",
        });
        nodeUpdateSampleLabel(fixture.path, 1, "alpha-updated");

        const rows = SqliteDatabase.open(fixture.path).table("samples", READ_LIMITS);
        assert.equal(rows.length, 3);
        assert.equal(rowById(rows, 1n).columns[3].valueText, "alpha-updated");
        assert.equal(rowById(rows, 3n).columns[1].valueInteger, 99n);
        assert.equal(rowById(rows, 3n).columns[3].valueText, "gamma");
        assert.equal(rowById(rows, 3n).columns[5].valueText, "added");
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});

test("node:sqlite case-table modifications remain visible to @yydb/sqlite", { timeout: 20_000 }, () => {
    const fixture = createNodeCaseFixture();
    try {
        nodeInsertCase(fixture.path, { rowid: 3, label: "第三" });

        const rows = SqliteDatabase.open(fixture.path).table("案例", READ_LIMITS);
        assert.equal(rows.length, 3);
        assert.equal(columnText(rowById(rows, 3n).columns[1]), "第三");
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});

test("a node:sqlite file is readable as a copied @yydb/sqlite snapshot", { timeout: 20_000 }, () => {
    const fixture = createNodeSamplesFixture();
    try {
        const snapshot = new SqliteSnapshot(new Uint8Array(fixture.bytes));
        const rows = snapshot.tableDecoded("samples", READ_LIMITS);
        assert.equal(rows.length, 2);
        assert.equal(rows[0].columns[3].valueText, "alpha");
        assert.deepEqual(snapshot.pages().subarray(0, MAGIC.length), MAGIC);
        assert.deepEqual(readFileSync(fixture.path), fixture.bytes);
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});

test("node:sqlite rows decode consistently through file and snapshot readers", { timeout: 20_000 }, () => {
    const fixture = createNodeSamplesFixture();
    const expected = [
        { rowid: 1, quantity: 42, score: 3.5, label: "alpha", payload: Buffer.from([0x00, 0xff, 0x10]), note: null },
        { rowid: 2, quantity: -7, score: -0.25, label: "beta", payload: Buffer.from([0xff]), note: "second" },
    ];
    try {
        const fileRows = SqliteDatabase.open(fixture.path).table("samples", READ_LIMITS);
        const snapshotRows = new SqliteSnapshot(new Uint8Array(fixture.bytes)).tableDecoded("samples", READ_LIMITS);
        assertSamplesMatchYyds(fileRows, expected);
        assertSamplesMatchYyds(snapshotRows, expected);
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});
