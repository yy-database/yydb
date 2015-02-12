import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import test from "node:test";

import { SqliteDatabase, SqliteSnapshot } from "@yydb/sqlite";

const MAGIC = Buffer.from("SQLite format 3\0", "latin1");
const PAGE_SIZE_DIRECTORY = { maxPages: 64, maxRows: 16 } as const;

interface Fixture {
    directory: string;
    path: string;
    bytes: Buffer;
}

/**
 * Writes the fixture with the host `node:sqlite` implementation, so the
 * snapshot reader is verified against a real SQLite file rather than a
 * repository asset.
 */
function createFixture(): Fixture {
    const directory = mkdtempSync(join(tmpdir(), "yydb-node-sqlite-"));
    const path = join(directory, "samples.sqlite");
    const database = new DatabaseSync(path);
    try {
        database.exec(
            "CREATE TABLE samples (id INTEGER PRIMARY KEY, quantity INTEGER, score REAL, label TEXT, payload BLOB, note TEXT)",
        );
        database.exec(
            "INSERT INTO samples (id, quantity, score, label, payload, note) VALUES (1, 42, 3.5, 'alpha', x'00ff10', NULL)",
        );
        database.exec(
            "INSERT INTO samples (id, quantity, score, label, payload, note) VALUES (2, -7, -0.25, 'beta', x'ff', 'second')",
        );
    } finally {
        database.close();
    }
    return { directory, path, bytes: readFileSync(path) };
}

function assertRowid(rowid: unknown, expected: bigint): void {
    assert.equal(rowid, expected, `rowid must be the bigint ${expected}`);
}

test("node:sqlite rows decode through the @yydb/sqlite reader", { timeout: 20_000 }, () => {
    const fixture = createFixture();
    try {
        const database = SqliteDatabase.open(fixture.path);
        assert.equal(database.path(), fixture.path);

        const schema = database.schema(PAGE_SIZE_DIRECTORY);
        const samples = schema.find((object) => object.name === "samples");
        assert.ok(samples, "schema must contain the samples table");
        assert.equal(samples.kind, "table", "samples must be reported as an ordinary table");

        const rows = database.table("samples", PAGE_SIZE_DIRECTORY);
        assert.equal(rows.length, 2, "both inserted rows must be readable");

        const [first, second] = rows;
        assertRowid(first.rowid, 1n);
        assertRowid(second.rowid, 2n);

        const [id, quantity, score, label, payload, note] = first.columns;
        assert.equal(
            id.kind,
            "null",
            "an INTEGER PRIMARY KEY is a rowid alias, so its record column stores NULL",
        );
        assert.equal(quantity.kind, "integer", "INTEGER column must decode as the integer storage class");
        assert.equal(quantity.valueInteger, 42n, "INTEGER column must decode to a bigint value");
        assert.equal(score.kind, "real", "REAL column must decode as the real storage class");
        assert.equal(score.valueReal, 3.5, "REAL column must decode to a number value");
        assert.equal(label.kind, "text", "TEXT column must decode as the text storage class");
        assert.equal(label.valueText, "alpha", "TEXT column must decode to a string value");
        assert.equal(payload.kind, "blob", "BLOB column must decode as the blob storage class");
        assert.deepEqual(payload.valueBlob, Buffer.from([0x00, 0xff, 0x10]), "BLOB column must decode to the original bytes");
        assert.equal(note.kind, "null", "NULL column must decode as the null storage class");
        assert.equal(note.valueText ?? null, null, "NULL column must not carry a text value");
        assert.equal(note.valueBlob ?? null, null, "NULL column must not carry a blob value");

        const [, secondQuantity, , , secondPayload, secondNote] = second.columns;
        assert.equal(secondQuantity.valueInteger, -7n, "negative INTEGER must decode to a bigint value");
        assert.deepEqual(secondPayload.valueBlob, Buffer.from([0xff]), "second BLOB must decode to the original bytes");
        assert.equal(secondNote.kind, "text", "second note must decode as text");
        assert.equal(secondNote.valueText, "second", "second note must decode to a string value");

        assert.deepEqual(
            database.pages().subarray(0, MAGIC.length),
            MAGIC,
            "main-file bytes must start with the SQLite format-3 magic header",
        );
        const version = database.sqliteVersion();
        assert.equal(typeof version, "string", "sqliteVersion must return a string");
        assert.ok(version.length > 0, "sqliteVersion must not be empty");
        assert.equal(database.ping(), 1, "ping must re-validate the main file");

        assert.throws(
            () => database.table("samples", { maxRows: 1 }),
            /row limit/,
            "row limits must bound the scan",
        );
        assert.deepEqual(
            readFileSync(fixture.path),
            fixture.bytes,
            "opening a snapshot must not modify the source file",
        );
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});

test("a node:sqlite file is readable as a copied snapshot", { timeout: 20_000 }, () => {
    const fixture = createFixture();
    try {
        const options = {
            maxPages: PAGE_SIZE_DIRECTORY.maxPages,
            maxRows: PAGE_SIZE_DIRECTORY.maxRows,
        };
        const snapshot = new SqliteSnapshot(new Uint8Array(fixture.bytes));
        const schema = snapshot.schema(options);
        assert.ok(
            schema.some((object) => object.name === "samples"),
            "snapshot schema must contain the samples table",
        );
        const rows = snapshot.tableDecoded("samples", options);
        assert.equal(rows.length, 2, "snapshot must expose both rows");
        assert.equal(rows[0].columns[3].valueText, "alpha", "snapshot text column must decode to a string");
        assert.deepEqual(
            snapshot.pages().subarray(0, MAGIC.length),
            MAGIC,
            "snapshot bytes must keep the SQLite format-3 magic header",
        );
        assert.deepEqual(
            readFileSync(fixture.path),
            fixture.bytes,
            "snapshot construction must not modify the source file",
        );
    } finally {
        rmSync(fixture.directory, { recursive: true, force: true });
    }
});
