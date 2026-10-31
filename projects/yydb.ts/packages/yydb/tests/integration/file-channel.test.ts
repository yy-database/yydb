import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { Database, type FileScalar } from "@yydb/yydb";

const ASSET_SCHEMA = `table Asset {
    @@asset_id: i64,
    payload: file,
}`;

const INSERT_ASSET = "Asset::insert({ asset_id: $id, payload: $payload })";

function payload(len: number): Uint8Array {
    return Uint8Array.from({ length: len }, (_, index) => index % 251);
}

function hexOf(octets: Uint8Array): string {
    return Buffer.from(octets).toString("hex");
}

function blobPath(directory: string, stem: string, hashHex: string): string {
    return join(directory, `${stem}-objects`, hashHex.slice(0, 2), `${hashHex}.blob`);
}

test("@yydb/yydb publishes a file value and reads it back through the public handle", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-file-channel-"));
    const path = join(directory, "assets.yydx");
    try {
        const database = await Database.open(path);
        await database.ensureSchema(ASSET_SCHEMA);
        const octets = payload(1000);
        const file = await database.putFile(octets, { chunkSize: 256, name: "asset.bin", mediaType: "application/octet-stream" });

        assert.equal(file.$yydb, "file");
        assert.match(file.digest, /^[0-9a-f]{64}$/);
        assert.equal(file.octets, octets.length);
        assert.equal(file.name, "asset.bin");
        assert.equal(file.mediaType, "application/octet-stream");
        assert.match(file.manifest.hash, /^[0-9a-f]{64}$/);

        // Renaming never changes content identity: same octets, same manifest.
        const renamed = await database.putFile(octets, { chunkSize: 256, name: "other.bin" });
        assert.equal(renamed.digest, file.digest);
        assert.equal(renamed.manifest.hash, file.manifest.hash);

        await database.execute(INSERT_ASSET, { id: 1, payload: file });
        const rows = await database.query<{ asset_id: number; payload: FileScalar }>("Asset.all().collect()");
        assert.equal(rows.length, 1);
        assert.deepEqual(rows[0].payload, file, "a queried file column must equal the published descriptor");

        assert.equal(hexOf(await database.readFile(file)), hexOf(octets));
        assert.equal(hexOf(await database.readFileWindow(file, 200, 200)), hexOf(octets.slice(200, 400)));
        assert.equal(hexOf(await database.readFileWindow(file, 900, 500)), hexOf(octets.slice(900)), "a window past the end is a short read");
        assert.equal((await database.readFileWindow(file, 1000, 10)).length, 0, "an offset at end of file is an empty window");
        assert.equal((await database.readFileWindow(file, 1500, 10)).length, 0, "an offset past end of file is an empty window");

        await database.close();

        const reopened = await Database.open(path);
        try {
            const [row] = await reopened.query<{ asset_id: number; payload: FileScalar }>("Asset.all().collect()");
            assert.equal(hexOf(await reopened.readFile(row.payload)), hexOf(octets), "reopen must re-read every chunk from disk");
        } finally {
            await reopened.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("@yydb/yydb refuses file values on a single-file .yydb", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-file-single-"));
    const path = join(directory, "assets.yydb");
    try {
        const database = await Database.open(path);
        await assert.rejects(() => database.putFile(payload(32)), /file|yydx/i, "single-file `.yydb` must reject `file` values");
        await database.close();
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("@yydb/yydb fails closed when a manifest object disappears", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-file-tamper-"));
    const path = join(directory, "assets.yydx");
    try {
        const database = await Database.open(path);
        const octets = payload(600);
        const file = await database.putFile(octets, { chunkSize: 256 });
        await database.close();

        rmSync(blobPath(directory, "assets", file.manifest.hash));

        const reopened = await Database.open(path);
        try {
            await assert.rejects(() => reopened.readFile(file), /not found|missing/i, "a missing manifest must not read as empty");
            await assert.rejects(() => reopened.readFileWindow(file, 0, 10), /not found|missing/i);
        } finally {
            await reopened.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});
