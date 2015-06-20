import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import test from "node:test";

import { SqliteDatabase } from "@yydb/sqlite";
import {
    disposeReferenceFixture,
    EXPECTED_COLUMNS,
    generateReferenceFixture,
    headerEncoding,
    matchesReference,
    referencePython,
} from "@yydb/sqlite-fixtures";

const SKIP_NO_PYTHON = referencePython() ? false : "no python3/python with sqlite3 on this host";
const PAGE_SIZES = [1024, 4096] as const;
const READ_LIMITS = { maxPages: 256, maxRows: 1024, maxPayloadBytes: 1_048_576, maxTotalPayloadBytes: 16_777_216 };

const ROWID_TABLES = ["simple", "implicit", "empty_table", "types", "multi", "affinity", "bulk"] as const;

function schemaKindName(kind: string): string {
    return kind;
}

test("reference corpus schema matches upstream SQLite for both page sizes", { timeout: 60_000, skip: SKIP_NO_PYTHON }, () => {
    for (const pageSize of PAGE_SIZES) {
        const fixture = generateReferenceFixture(pageSize);
        assert.ok(fixture, "fixture generation must succeed when Python is available");
        try {
            assert.equal(fixture.pageSize, pageSize);
            assert.equal(fixture.manifest.pageSize, pageSize);
            assert.equal(fixture.manifest.encoding, headerEncoding(fixture.bytes));

            const database = SqliteDatabase.open(fixture.path);
            const objects = database.schema(READ_LIMITS);
            assert.equal(objects.length, fixture.manifest.schema.length, `schema count @ ${pageSize}`);

            for (const [index, object] of objects.entries()) {
                const reference = fixture.manifest.schema[index];
                assert.equal(schemaKindName(object.kind), reference.kind);
                assert.equal(object.name, reference.name);
                assert.equal(object.tableName, reference.tableName);
                assert.equal(object.rootPage, reference.rootPage);
                assert.equal(object.sql ?? null, reference.sql, `CREATE source for ${object.name}`);
            }

            for (const [table, expected] of Object.entries(EXPECTED_COLUMNS)) {
                const actual = fixture.manifest.columns.filter((column) => column.table === table);
                assert.equal(actual.length, expected.length, `column count for ${table} @ ${pageSize}`);
                for (const [column, want] of actual.entries()) {
                    assert.equal(column.cid, want[0]);
                    assert.equal(column.name, want[1]);
                    assert.equal(column.declaredType, want[2]);
                    assert.equal(column.notNull, want[3]);
                    assert.equal(column.default, want[4]);
                    assert.equal(column.primaryKey, want[5]);
                }
            }

            assert.ok(objects.some((object) => object.kind === "index" && object.name === "multi_amount_idx"));
            assert.ok(objects.some((object) => object.kind === "view" && object.name === "v_simple"));
            assert.ok(objects.some((object) => object.kind === "index" && (object.sql ?? null) === null));
        } finally {
            disposeReferenceFixture(fixture);
        }
    }
});

test("reference corpus rowid scans match upstream SQLite", { timeout: 60_000, skip: SKIP_NO_PYTHON }, () => {
    for (const pageSize of PAGE_SIZES) {
        const fixture = generateReferenceFixture(pageSize);
        assert.ok(fixture);
        try {
            const database = SqliteDatabase.open(fixture.path);
            for (const tableName of ROWID_TABLES) {
                if (tableName === "affinity") {
                    continue;
                }
                const reference = fixture.manifest.tables.find((table) => table.name === tableName);
                assert.ok(reference, `manifest table ${tableName}`);
                const rows = database.table(tableName, READ_LIMITS);
                assert.equal(rows.length, reference.rows.length, `row count for ${tableName} @ ${pageSize}`);

                for (const [index, row] of rows.entries()) {
                    const expected = reference.rows[index];
                    assert.equal(row.rowid, BigInt(expected.rowid), `rowid ${tableName}`);
                    for (const [columnIndex, column] of row.columns.entries()) {
                        assert.ok(
                            matchesReference(column, expected.values[columnIndex]),
                            `${tableName} rowid ${row.rowid} column ${columnIndex}`,
                        );
                    }
                }
            }
        } finally {
            disposeReferenceFixture(fixture);
        }
    }
});

test("affinity exposes on-disk integer storage rather than SQL-projected REAL", { timeout: 60_000, skip: SKIP_NO_PYTHON }, () => {
    const fixture = generateReferenceFixture(4096);
    assert.ok(fixture);
    try {
        const reference = fixture.manifest.tables.find((table) => table.name === "affinity");
        assert.ok(reference);
        assert.deepEqual(reference.rows[0].values[1], { kind: "real", bits: 0n });

        const rows = SqliteDatabase.open(fixture.path).table("affinity", READ_LIMITS);
        assert.equal(rows[0].columns[1].kind, "integer");
        assert.equal(rows[0].columns[1].valueInteger, 0n);

        const node = new DatabaseSync(fixture.path, { readOnly: true });
        const projected = node.prepare('SELECT v FROM affinity').get() as { v: number };
        node.close();
        assert.equal(projected.v, 0);
    } finally {
        disposeReferenceFixture(fixture);
    }
});

test("node:sqlite reads the same corpus rows as @yydb/sqlite", { timeout: 60_000, skip: SKIP_NO_PYTHON }, () => {
    const fixture = generateReferenceFixture(4096);
    assert.ok(fixture);
    const node = new DatabaseSync(fixture.path, { readOnly: true });
    try {
        const yyds = SqliteDatabase.open(fixture.path);

        for (const tableName of ["simple", "types"] as const) {
            const yydsRows = yyds.table(tableName, READ_LIMITS);
            // The corpus stores `i64::MIN` and `i64::MAX`, which `node:sqlite`
            // only returns as BigInt values.
            const statement = node.prepare(`SELECT id AS rowid, * FROM "${tableName}" ORDER BY rowid`);
            statement.setReadBigInts(true);
            const nodeRows = statement.all() as Array<Record<string, unknown>>;
            assert.equal(yydsRows.length, nodeRows.length, tableName);
            for (const [index, row] of yydsRows.entries()) {
                assert.equal(row.rowid, nodeRows[index].rowid, `${tableName} rowid ${index}`);
            }
        }
    } finally {
        node.close();
        disposeReferenceFixture(fixture);
    }
});
