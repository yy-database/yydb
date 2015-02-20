import assert from "node:assert/strict";
import { rmSync } from "node:fs";
import test from "node:test";

import { SqliteDatabase } from "@yydb/sqlite";

import {
    readCaseWithNode,
    readSamplesWithNode,
} from "./fixtures/node-sqlite.ts";
import { MAGIC, READ_LIMITS, SAMPLES_ROW_PAYLOADS } from "./fixtures/shared.ts";
import { rewriteYydsCaseFixture, rewriteYydsSamplesFixture, writeYydsCaseFixture, writeYydsSamplesFixture } from "./fixtures/yyds-sqlite.ts";

test("yyds-built samples bytes are readable by node:sqlite", { timeout: 20_000 }, () => {
    const fixture = writeYydsSamplesFixture([
        { rowid: 1 },
        { rowid: 2 },
    ]);
    try {
        const rows = readSamplesWithNode(fixture.path);
        assert.equal(rows.length, 2);
        assert.equal(rows[0].quantity, 42);
        assert.equal(rows[0].score, 3.5);
        assert.equal(rows[0].label, "alpha");
        assert.deepEqual(rows[0].payload, Buffer.from([0x00, 0xff, 0x10]));
        assert.equal(rows[0].note, null);
        assert.equal(rows[1].quantity, -7);
        assert.equal(rows[1].label, "beta");
        assert.equal(rows[1].note, "second");

        const reader = SqliteDatabase.open(fixture.path);
        assert.equal(reader.ping(), 1);
        assert.deepEqual(reader.pages().subarray(0, MAGIC.length), MAGIC);
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});

test("yyds-built 案例 bytes are readable by node:sqlite", { timeout: 20_000 }, () => {
    const fixture = writeYydsCaseFixture([
        { rowid: 1, label: "第一" },
        { rowid: 2, label: "第二" },
    ]);
    try {
        const rows = readCaseWithNode(fixture.path);
        assert.deepEqual(rows, [
            { rowid: 1, label: "第一" },
            { rowid: 2, label: "第二" },
        ]);

        const schema = SqliteDatabase.open(fixture.path).schema(READ_LIMITS);
        assert.ok(schema.some((object) => object.name === "案例" && object.kind === "table"));
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});

test("yyds rewrites remain visible to node:sqlite", { timeout: 20_000 }, () => {
    const fixture = writeYydsSamplesFixture([{ rowid: 1 }, { rowid: 2 }]);
    try {
        rewriteYydsSamplesFixture(fixture.path, [
            { rowid: 1 },
            { rowid: 2 },
            { rowid: 3 },
        ]);

        const rows = readSamplesWithNode(fixture.path);
        assert.equal(rows.length, 3);
        assert.equal(rows[2].rowid, 3);
        assert.equal(rows[2].quantity, 99);
        assert.equal(rows[2].score, 1.25);
        assert.equal(rows[2].label, "gamma");
        assert.deepEqual(rows[2].payload, Buffer.from([0xab]));
        assert.equal(rows[2].note, "added");
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});

test("yyds case-table rewrites remain visible to node:sqlite", { timeout: 20_000 }, () => {
    const fixture = writeYydsCaseFixture([
        { rowid: 1, label: "第一" },
        { rowid: 2, label: "第二" },
    ]);
    try {
        rewriteYydsCaseFixture(fixture.path, [
            { rowid: 1, label: "第一-改" },
            { rowid: 2, label: "第二" },
            { rowid: 3, label: "第三" },
        ]);

        const rows = readCaseWithNode(fixture.path);
        assert.deepEqual(rows, [
            { rowid: 1, label: "第一-改" },
            { rowid: 2, label: "第二" },
            { rowid: 3, label: "第三" },
        ]);
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});
