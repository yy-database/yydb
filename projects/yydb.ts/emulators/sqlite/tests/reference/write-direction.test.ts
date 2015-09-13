import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { SqliteWriter } from "@yydb/sqlite";
import {
    openBetterSqlite3,
    readWithBetterSqlite3,
    readWithPython,
    referencePython,
    type ExternalReport,
    type ExternalValue,
} from "@yydb/sqlite-fixtures";

const SKIP_NO_PYTHON = referencePython() ? false : "no python3/python with sqlite3 on this host";
const I64_MAX = 9223372036854775807n;
const I64_MIN = -9223372036854775808n;

const LARGE_TEXT = "甲乙丙丁戊己庚辛壬癸-external-".repeat(1_200); // ~43 KB, needs overflow pages

interface ExpectedRow {
    rowid: bigint;
    values: ExternalValue[];
}

function text(value: string): ExternalValue {
    return { kind: "text", value };
}

function integer(value: bigint | number): ExternalValue {
    return { kind: "integer", value: BigInt(value) };
}

function real(value: number): ExternalValue {
    const buffer = Buffer.alloc(8);
    buffer.writeDoubleBE(value);
    return { kind: "real", bits: buffer.readBigUInt64BE() };
}

function blob(bytes: Uint8Array): ExternalValue {
    return { kind: "blob", value: Buffer.from(bytes) };
}

const NULL: ExternalValue = { kind: "null" };

function buildCorpus(path: string): { expected: ExpectedRow[] } {
    const writer = SqliteWriter.create(4096);
    writer.createTable("items", [
        { name: "id", declaredType: "INTEGER", primaryKey: true },
        { name: "sku", declaredType: "TEXT", unique: true },
        { name: "name", declaredType: "TEXT", notNull: true, defaultValue: "anon" },
        { name: "note", declaredType: "TEXT" },
        { name: "score", declaredType: "REAL" },
        { name: "payload", declaredType: "BLOB" },
    ]);
    writer.createTable("empty_table", [
        { name: "id", declaredType: "INTEGER", primaryKey: true },
        { name: "v", declaredType: "TEXT" },
    ]);
    writer.createIndex("items_score", "items", ["score"], false);

    const expected: ExpectedRow[] = [];
    for (let index = 1; index <= 300; index += 1) {
        const note = index % 3 === 0 ? null : `note-${index}-备注`;
        const score = index % 2 === 0 ? index + 0.5 : null;
        const payload = new Uint8Array([index % 256, 255 - (index % 256), index % 7]);
        writer.insert("items", [index, `sku-${index}`, `item-${index}-中文`, note, score, payload]);
        expected.push({
            rowid: BigInt(index),
            values: [integer(index), text(`sku-${index}`), text(`item-${index}-中文`), note === null ? NULL : text(note), score === null ? NULL : real(score), blob(payload)],
        });
    }
    // Edge rows: boundary integers, default application, large overflow text,
    // empty strings, negative rowid, and a NULL-heavy row.
    writer.insert("items", [I64_MAX, "sku-max", "edge-max", null, 1e300, new Uint8Array([0])]);
    expected.push({ rowid: I64_MAX, values: [integer(I64_MAX), text("sku-max"), text("edge-max"), NULL, real(1e300), blob(new Uint8Array([0]))] });
    writer.insert("items", [I64_MIN, "sku-min", undefined, null, -0.5, null]);
    expected.push({ rowid: I64_MIN, values: [integer(I64_MIN), text("sku-min"), text("anon"), NULL, real(-0.5), NULL] });
    writer.insert("items", [-5, "sku-neg", "负数行", "", -12345, LARGE_TEXT_AS_BLOB]);
    // REAL affinity converts the integer score to a double before storage.
    expected.push({ rowid: -5n, values: [integer(-5), text("sku-neg"), text("负数行"), text(""), real(-12345), blob(LARGE_TEXT_AS_BLOB)] });
    writer.insert("items", [400, "sku-big", LARGE_TEXT, null, null, null]);
    expected.push({ rowid: 400n, values: [integer(400), text("sku-big"), text(LARGE_TEXT), NULL, NULL, NULL] });

    // Update and delete leave freeblocks; the file must stay externally valid.
    writer.update("items", 5, [{ keep: true }, { keep: true }, { set: "updated-5" }, { keep: true }, { keep: true }, { keep: true }]);
    const fifth = expected.find((row) => row.rowid === 5n);
    if (fifth) fifth.values[2] = text("updated-5");
    writer.delete("items", 6);
    expected.splice(expected.findIndex((row) => row.rowid === 6n), 1);

    writer.save(path);
    expected.sort((left, right) => (left.rowid < right.rowid ? -1 : left.rowid > right.rowid ? 1 : 0));
    return { expected };
}

const LARGE_TEXT_AS_BLOB = new TextEncoder().encode(LARGE_TEXT);

function assertReportMatches(report: ExternalReport, expected: ExpectedRow[], label: string) {
    assert.equal(report.integrity, "ok", `${label}: PRAGMA integrity_check`);
    assert.equal(report.pageSize, 4096, `${label}: page size`);

    const table = report.schema.find((object) => object.kind === "table" && object.name === "items");
    assert.ok(table, `${label}: items table object`);
    assert.equal(table.sql, 'CREATE TABLE "items" ("id" INTEGER PRIMARY KEY, "sku" TEXT UNIQUE, "name" TEXT NOT NULL DEFAULT \'anon\', "note" TEXT, "score" REAL, "payload" BLOB)', `${label}: CREATE TABLE text`);
    assert.ok(report.schema.some((object) => object.kind === "table" && object.name === "empty_table"), `${label}: empty_table object`);
    assert.ok(report.schema.some((object) => object.kind === "index" && object.name === "items_score"), `${label}: index object`);
    assert.ok(
        report.schema.some((object) => object.kind === "index" && object.name === "sqlite_autoindex_items_1"),
        `${label}: UNIQUE constraint autoindex object`,
    );

    const columns = report.columns.items;
    assert.ok(columns, `${label}: items columns`);
    assert.deepEqual(
        columns.map((column) => [column.name, column.declaredType, column.notNull, column.primaryKey]),
        [
            ["id", "INTEGER", false, 1],
            ["sku", "TEXT", false, 0],
            ["name", "TEXT", true, 0],
            ["note", "TEXT", false, 0],
            ["score", "REAL", false, 0],
            ["payload", "BLOB", false, 0],
        ],
        `${label}: column names, types, NOT NULL and pk flags`,
    );
    assert.equal(columns[2].default, "'anon'", `${label}: DEFAULT survives as SQL text`);

    const indexNames = report.indexes.items.map((index) => index.name).sort();
    assert.deepEqual(indexNames, ["items_score", "sqlite_autoindex_items_1"], `${label}: index list`);

    assert.deepEqual(report.tables.empty_table, [], `${label}: empty table reads as zero rows`);

    const rows = report.tables.items;
    assert.equal(rows.length, expected.length, `${label}: row count`);
    for (const [position, row] of rows.entries()) {
        const want = expected[position];
        assert.equal(row.rowid, want.rowid, `${label}: rowid at position ${position}`);
        assert.equal(row.values.length, want.values.length, `${label}: column count at rowid ${want.rowid}`);
        for (const [columnIndex, value] of row.values.entries()) {
            assert.deepEqual(value, want.values[columnIndex], `${label}: rowid ${want.rowid} column ${columnIndex}`);
        }
    }
}

test("writer output is readable by Python sqlite3 with structure and content intact", { timeout: 120_000, skip: SKIP_NO_PYTHON }, () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-write-python-"));
    const path = join(directory, "corpus.sqlite");
    try {
        const { expected } = buildCorpus(path);
        const report = readWithPython(path);
        assert.ok(report, "python reader must run when python is available");
        assertReportMatches(report, expected, "python");
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("writer output is readable by better-sqlite3 with structure and content intact", { timeout: 120_000 }, async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-write-better-"));
    const path = join(directory, "corpus.sqlite");
    try {
        const { expected } = buildCorpus(path);
        const report = await readWithBetterSqlite3(path);
        assertReportMatches(report, expected, "better-sqlite3");
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("external implementations agree with each other on the writer output", { timeout: 120_000, skip: SKIP_NO_PYTHON }, async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-write-diff-"));
    const path = join(directory, "corpus.sqlite");
    try {
        buildCorpus(path);
        const python = readWithPython(path);
        const better = await readWithBetterSqlite3(path);
        assert.ok(python);
        assert.equal(python.integrity, better.integrity);
        assert.equal(python.pageSize, better.pageSize);
        assert.deepEqual(python.tables.items, better.tables.items, "both implementations read identical rows");
        assert.deepEqual(python.columns.items, better.columns.items, "both implementations read identical columns");
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("unique constraints stay enforced for external writers", { timeout: 120_000, skip: SKIP_NO_PYTHON }, async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-write-unique-"));
    const path = join(directory, "corpus.sqlite");
    try {
        buildCorpus(path);

        const python = referencePython();
        assert.ok(python);
        const { spawnSync } = await import("node:child_process");
        const probe = spawnSync(
            python,
            [
                "-c",
                "import sqlite3, sys\nconnection = sqlite3.connect(sys.argv[1])\ntry:\n    connection.execute(\"INSERT INTO items (id, sku, name) VALUES (99999, 'sku-1', 'dup')\")\n    connection.commit()\n    print('accepted')\nexcept sqlite3.IntegrityError:\n    print('rejected')\nfinally:\n    connection.close()",
                path,
            ],
            { encoding: "utf8", timeout: 30_000 },
        );
        assert.equal(probe.status, 0, probe.stderr);
        assert.equal(probe.stdout.trim(), "rejected", "python insert against the UNIQUE index must fail");

        const db = await openBetterSqlite3(path);
        try {
            assert.throws(
                () => db.prepare("INSERT INTO items (id, sku, name) VALUES (99999, 'sku-1', 'dup')").run(),
                /UNIQUE/i,
                "better-sqlite3 insert against the UNIQUE index must fail",
            );
        } finally {
            db.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("rolled-back transactions leave an externally consistent file", { timeout: 120_000, skip: SKIP_NO_PYTHON }, () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-write-rollback-"));
    const path = join(directory, "rollback.sqlite");
    try {
        const writer = SqliteWriter.create(4096);
        writer.createTable("ledger", [
            { name: "id", declaredType: "INTEGER", primaryKey: true },
            { name: "entry", declaredType: "TEXT" },
        ]);
        writer.insert("ledger", [1, "before-txn"]);
        writer.begin();
        writer.insert("ledger", [2, "inside-txn"]);
        writer.update("ledger", 1, [{ keep: true }, { set: "mutated-inside-txn" }]);
        writer.delete("ledger", 1);
        writer.rollback();
        writer.save(path);

        const report = readWithPython(path);
        assert.ok(report);
        assert.equal(report.integrity, "ok");
        assert.deepEqual(report.tables.ledger, [
            { rowid: 1n, values: [integer(1), text("before-txn")] },
        ], "rollback restores the exact pre-transaction state");
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("deleted rows and hollow pages stay externally readable", { timeout: 120_000, skip: SKIP_NO_PYTHON }, () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-write-hollow-"));
    const path = join(directory, "hollow.sqlite");
    try {
        // Small pages so deletes hollow out whole pages into the freelist.
        const writer = SqliteWriter.create(512);
        writer.createTable("bulk", [
            { name: "id", declaredType: "INTEGER", primaryKey: true },
            { name: "label", declaredType: "TEXT" },
        ]);
        for (let index = 1; index <= 200; index += 1) {
            writer.insert("bulk", [index, `row-${index}`]);
        }
        const survivors: ExpectedRow[] = [];
        for (let index = 1; index <= 200; index += 1) {
            if (index % 7 === 3 || index > 140) {
                writer.delete("bulk", index);
                continue;
            }
            survivors.push({ rowid: BigInt(index), values: [integer(index), text(`row-${index}`)] });
        }
        writer.save(path);

        const report = readWithPython(path);
        assert.ok(report);
        assert.equal(report.integrity, "ok", "hollowed file passes integrity_check");
        assert.ok(report.freelistCount > 0, "emptied pages reach the freelist");
        assert.deepEqual(report.tables.bulk, survivors, "only surviving rows remain");
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});
