import assert from "node:assert/strict";
import { rmSync } from "node:fs";
import test from "node:test";

import { SqliteDatabase } from "@yydb/sqlite";

import {
    createNodeCaseFixture,
    createNodeSamplesFixture,
    nodeInsertCase,
    nodeUpdateSampleLabel,
    readCaseWithNode,
    readSamplesWithNode,
} from "./fixtures/node-sqlite.ts";
import { columnText, READ_LIMITS, rowById } from "./fixtures/shared.ts";
import { rewriteYydsCaseFixture, rewriteYydsSamplesFixture, writeYydsCaseFixture, writeYydsSamplesFixture } from "./fixtures/yyds-sqlite.ts";

test("node creates, yyds reads, node mutates, yyds reads again", { timeout: 20_000 }, () => {
    const fixture = createNodeSamplesFixture();
    try {
        let rows = SqliteDatabase.open(fixture.path).table("samples", READ_LIMITS);
        assert.equal(rowById(rows, 1n).columns[3].valueText, "alpha");

        nodeUpdateSampleLabel(fixture.path, 1, "mutated-by-node");

        rows = SqliteDatabase.open(fixture.path).table("samples", READ_LIMITS);
        assert.equal(rowById(rows, 1n).columns[3].valueText, "mutated-by-node");
        assert.equal(readSamplesWithNode(fixture.path)[0].label, "mutated-by-node");
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});

test("yyds creates, node reads, yyds mutates, node reads again", { timeout: 20_000 }, () => {
    const fixture = writeYydsCaseFixture([{ rowid: 1, label: "初始" }]);
    try {
        assert.deepEqual(readCaseWithNode(fixture.path), [{ rowid: 1, label: "初始" }]);

        rewriteYydsCaseFixture(fixture.path, [
            { rowid: 1, label: "初始-改" },
            { rowid: 2, label: "追加" },
        ]);

        assert.deepEqual(readCaseWithNode(fixture.path), [
            { rowid: 1, label: "初始-改" },
            { rowid: 2, label: "追加" },
        ]);
        assert.equal(columnText(rowById(SqliteDatabase.open(fixture.path).table("案例", READ_LIMITS), 2n).columns[1]), "追加");
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});

test("yyds creates samples, node mutates, yyds reads the node-side change", { timeout: 20_000 }, () => {
    const fixture = writeYydsSamplesFixture([{ rowid: 1 }, { rowid: 2 }]);
    try {
        nodeUpdateSampleLabel(fixture.path, 2, "beta-from-node");

        const rows = SqliteDatabase.open(fixture.path).table("samples", READ_LIMITS);
        assert.equal(rowById(rows, 2n).columns[3].valueText, "beta-from-node");
        assert.equal(readSamplesWithNode(fixture.path)[1].label, "beta-from-node");
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});

test("node creates 案例, yyds reads, node mutates, yyds reads again", { timeout: 20_000 }, () => {
    const fixture = createNodeCaseFixture();
    try {
        let rows = SqliteDatabase.open(fixture.path).table("案例", READ_LIMITS);
        assert.equal(columnText(rowById(rows, 1n).columns[1]), "第一");

        nodeInsertCase(fixture.path, { rowid: 3, label: "由-node-追加" });

        rows = SqliteDatabase.open(fixture.path).table("案例", READ_LIMITS);
        assert.equal(columnText(rowById(rows, 3n).columns[1]), "由-node-追加");
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});

test("yyds creates samples, yyds mutates, node still reads the yyds-side change", { timeout: 20_000 }, () => {
    const fixture = writeYydsSamplesFixture([{ rowid: 1 }]);
    try {
        rewriteYydsSamplesFixture(fixture.path, [{ rowid: 1 }, { rowid: 2 }]);
        assert.equal(readSamplesWithNode(fixture.path).length, 2);
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});
