import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

import { Database } from "@yydb/yydb";

const BLOB_SCHEMA = `table Blob {
    @@blob_id: i64,
    label: utf8,
    body: bytes,
}`;

const INSERT_BLOB = "Blob::insert({ blob_id: $id, label: $label, body: $body })";

/** The octet payloads one Node host must move through a `bytes` column verbatim. */
const PAYLOADS: ReadonlyArray<{ name: string; octets: Uint8Array }> = [
    { name: "empty", octets: new Uint8Array() },
    { name: "nul", octets: Uint8Array.from([0x00]) },
    { name: "invalid-utf8", octets: Uint8Array.from([0x00, 0x80, 0xff]) },
    { name: "all-byte-values", octets: Uint8Array.from({ length: 256 }, (_, index) => index) },
    { name: "inline-budget", octets: Uint8Array.from({ length: 1024 }, (_, index) => index % 256) },
    { name: "above-inline-budget", octets: Uint8Array.from({ length: 1025 }, (_, index) => index % 256) },
    { name: "four-kib", octets: Uint8Array.from({ length: 4096 }, (_, index) => index % 256) },
    { name: "four-kib-plus-one", octets: Uint8Array.from({ length: 4097 }, (_, index) => index % 256) },
];

/** Payloads small enough that the row keeps them inline instead of referencing an object. */
const INLINE_NAMES = new Set(["empty", "nul", "invalid-utf8", "all-byte-values", "inline-budget"]);

const here = dirname(fileURLToPath(import.meta.url));

async function storedRowJson(database: Database): Promise<Map<number, string>> {
    const rows = await database.scan({ prefix: "__yydb/v0/row/Blob/", limit: 1000 });
    return new Map(
        rows.map((row) => {
            const text = Buffer.from(row.value).toString("utf8");
            const parsed = JSON.parse(text) as { blob_id: number };
            return [Number(parsed.blob_id), text];
        }),
    );
}

function hexOf(octets: Uint8Array): string {
    return Buffer.from(octets).toString("hex");
}

test("@yydb/yydb moves octets through a VOS bytes column verbatim", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-bytes-channel-"));
    const path = join(directory, "bytes.yydx");
    try {
        const database = await Database.open(path);
        await database.ensureSchema(BLOB_SCHEMA);

        for (const [offset, payload] of PAYLOADS.entries()) {
            await database.execute(INSERT_BLOB, {
                id: offset + 1,
                label: payload.name,
                body: payload.octets,
            });
        }

        const rows = await database.query<{ blob_id: number; label: string; body: Uint8Array }>("Blob.all().collect()");
        assert.equal(rows.length, PAYLOADS.length);

        for (const [offset, payload] of PAYLOADS.entries()) {
            const row = rows[offset];
            assert.equal(Number(row.blob_id), offset + 1, payload.name);
            assert.equal(row.label, payload.name, payload.name);
            assert.ok(row.body instanceof Uint8Array, `${payload.name} must decode to octets, not text`);
            assert.equal(hexOf(row.body), hexOf(payload.octets), `${payload.name} octets must survive verbatim`);
        }

        // An empty payload is a stored value, not a SQL NULL.
        assert.notEqual(rows[0].body, null);

        // Placement is the storage host's decision: payloads inside the inline
        // row budget stay tagged `bytes`, larger ones become object references.
        const stored = await storedRowJson(database);
        assert.equal(stored.size, PAYLOADS.length);
        for (const [offset, payload] of PAYLOADS.entries()) {
            const row = stored.get(offset + 1);
            assert.ok(row, `${payload.name} must be stored`);
            const expected = INLINE_NAMES.has(payload.name) ? '"$yydb":"bytes"' : '"$yydb":"object"';
            assert.ok(row.includes(expected), `${payload.name} must be stored as ${expected}: ${row.slice(0, 120)}`);
        }

        await database.close();
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("@yydb/yydb reads the same octets back after the process exits", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-bytes-readback-"));
    const path = join(directory, "bytes.yydx");
    const readback = join(here, "fixtures", "bytes-readback.ts");
    try {
        const database = await Database.open(path);
        await database.ensureSchema(BLOB_SCHEMA);
        for (const [offset, payload] of PAYLOADS.entries()) {
            await database.execute(INSERT_BLOB, {
                id: offset + 1,
                label: payload.name,
                body: payload.octets,
            });
        }
        await database.close();

        const output = execFileSync(
            process.execPath,
            ["--experimental-strip-types", readback, path],
            { cwd: here, encoding: "utf8" },
        );
        const reopened = JSON.parse(output) as { id: number; hex: string }[];
        assert.equal(reopened.length, PAYLOADS.length);
        for (const [offset, payload] of PAYLOADS.entries()) {
            assert.equal(reopened[offset].id, offset + 1, payload.name);
            assert.equal(reopened[offset].hex, hexOf(payload.octets), `${payload.name} octets must survive a reopen`);
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});
