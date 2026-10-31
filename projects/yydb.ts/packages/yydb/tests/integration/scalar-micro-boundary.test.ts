import assert from "node:assert/strict";
import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { Database, YydbError, defineMicro, v } from "@yydb/yydb";

/**
 * Exact-type channels and the host micro path: the tagged wire forms for
 * `vector<N>` / `uuid` / `bytes` / `file`, their acceptance and rejection
 * boundaries, content-addressed file integrity across reopen, and the
 * Phase-1 micro definition and invocation surface.
 */

async function rejectDiagnostic(run: () => Promise<unknown>): Promise<YydbError> {
    let caught: unknown;
    try {
        await run();
    } catch (cause) {
        caught = cause;
    }
    assert.ok(caught, "expected the call to reject");
    if (caught instanceof YydbError) {
        return caught;
    }
    assert.ok(caught instanceof Error, `unexpected rejection: ${String(caught)}`);
    const diagnostic = YydbError.fromDiagnosticJson(caught.message);
    assert.ok(diagnostic, `rejection must carry a diagnostic envelope: ${caught.message}`);
    return diagnostic;
}

function vectorWire(values: readonly number[]): { $yydb: "vector"; dimension: number; components: string } {
    const floats = new Float32Array(values);
    return { $yydb: "vector", dimension: values.length, components: Buffer.from(floats.buffer).toString("hex") };
}

const VECTOR_SCHEMA = "table V {\n    @@v_id: i64,\n    emb: vector<3>,\n}";
const INSERT_VECTOR = "V::insert({ v_id: $id, emb: $e })";

test("a tagged `vector` parameter round-trips and survives a reopen", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-vector-"));
    const path = join(directory, "vector.yydb");
    try {
        const database = await Database.open(path);
        await database.ensureSchema(VECTOR_SCHEMA);
        const wire = vectorWire([1.5, -2.25, 0]);
        await database.execute(INSERT_VECTOR, { id: 1, e: wire });
        const rows = await database.query<{ v_id: number; emb: { $yydb: string; dimension: number; components: string } }>(
            "V.all().collect()",
        );
        assert.equal(rows.length, 1);
        assert.deepEqual(rows[0].emb, { $yydb: "vector", dimension: 3, components: wire.components });
        await database.close();

        const reopened = await Database.open(path);
        try {
            const again = await reopened.query<{ emb: { $yydb: string; dimension: number; components: string } }>("V.all().collect()");
            assert.deepEqual(again[0].emb, rows[0].emb);
        } finally {
            await reopened.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("a `vector` component count mismatch is refused with the expected dimension", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-vector-dim-"));
    try {
        const database = await Database.open(join(directory, "dim.yydb"));
        try {
            await database.ensureSchema(VECTOR_SCHEMA);
            const rejection = await rejectDiagnostic(() => database.execute(INSERT_VECTOR, { id: 1, e: vectorWire([1, 2]) }));
            assert.equal(rejection.code, "yydb.schema.invalid");
            assert.match(rejection.message, /requires 3 components, got 2/);
        } finally {
            await database.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("non-finite, malformed or truncated `vector` encodings are refused at the wire", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-vector-bad-"));
    try {
        const database = await Database.open(join(directory, "bad.yydb"));
        try {
            await database.ensureSchema(VECTOR_SCHEMA);
            await assert.rejects(
                database.execute(INSERT_VECTOR, { id: 1, e: { $yydb: "vector", dimension: 3, components: vectorWire([Number.NaN, 0, 0]).components } }),
                /component 0 is not finite/,
            );
            await assert.rejects(
                database.execute(INSERT_VECTOR, { id: 2, e: { $yydb: "vector", dimension: 3, components: "zzzz" } }),
                /invalid vector/,
            );
            await assert.rejects(
                database.execute(INSERT_VECTOR, { id: 3, e: { $yydb: "vector", dimension: 3, components: "0000c03f" } }),
                /expects 12 bytes, got 4/,
            );
            await assert.rejects(
                database.execute(INSERT_VECTOR, { id: 4, e: [1, 2, 3] }),
                /unsupported scalar value/,
            );
        } finally {
            await database.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("a `vector` value cannot be stored in a non-vector column", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-vector-col-"));
    try {
        const database = await Database.open(join(directory, "col.yydb"));
        try {
            await database.ensureSchema("table W {\n    @@w_id: i64,\n    count: i64,\n}");
            await assert.rejects(
                database.execute("W::insert({ w_id: 1, count: $c })", { c: vectorWire([1, 2, 3]) }),
                /VECTOR value cannot be stored/,
            );
        } finally {
            await database.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("an oversized fixed dimension is refused at schema publish time", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-vector-cap-"));
    try {
        const database = await Database.open(join(directory, "cap.yydb"));
        try {
            const rejection = await rejectDiagnostic(() => database.ensureSchema("table Big {\n    @@b_id: i64,\n    e: vector<16385>,\n}"));
            assert.equal(rejection.code, "yydb.schema.vector_dimension_exceeded");
        } finally {
            await database.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("embedding placement switches from inline to object between 120 and 121 dimensions", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-vector-place-"));
    try {
        const database = await Database.open(join(directory, "place.yydx"));
        try {
            await database.ensureSchema(
                "table V120 {\n    @@v_id: i64,\n    e: vector<120>,\n}\n\ntable V121 {\n    @@v_id: i64,\n    e: vector<121>,\n}",
            );
            await database.execute("V120::insert({ v_id: 1, e: $e })", { e: vectorWire(Array.from({ length: 120 }, (_, i) => i)) });
            await database.execute("V121::insert({ v_id: 1, e: $e })", { e: vectorWire(Array.from({ length: 121 }, (_, i) => i)) });

            const inline = await database.scan({ prefix: "__yydb/v0/row/V120/", limit: 10 });
            assert.ok(inline.length > 0);
            assert.match(Buffer.from(inline[0].value).toString("utf8"), /"\$yydb":"vector"/);

            const object = await database.scan({ prefix: "__yydb/v0/row/V121/", limit: 10 });
            assert.ok(object.length > 0);
            assert.match(Buffer.from(object[0].value).toString("utf8"), /"\$yydb":"object"/);
        } finally {
            await database.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("a `uuid` column stores the literal and reads back the tagged form", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-uuid-"));
    try {
        const database = await Database.open(join(directory, "uuid.yydb"));
        try {
            await database.ensureSchema("table U {\n    @@u_id: uuid,\n    label: utf8,\n}");
            const id = "0e7f3a9c-1b2d-4e5f-6a7b-8c9d0e1f2a3b";
            await database.execute('U::insert({ u_id: $id, label: "first" })', { id });
            const rows = await database.query<{ u_id: { $yydb: string; value: string }; label: string }>("U.all().collect()");
            assert.deepEqual(rows[0].u_id, { $yydb: "uuid", value: id });

            await assert.rejects(
                database.execute('U::insert({ u_id: $id, label: "bad" })', { id: "not-a-uuid" }),
                /requires a UUID literal/,
            );
        } finally {
            await database.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("an over-budget inline `bytes` payload is refused on the single-file layout", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-bytes-cap-"));
    try {
        const database = await Database.open(join(directory, "cap.yydb"));
        try {
            await database.ensureSchema("table B {\n    @@b_id: i64,\n    body: bytes,\n}");
            const octets = Uint8Array.from({ length: 1025 }, (_, i) => i % 256);
            const rejection = await rejectDiagnostic(() => database.execute("B::insert({ b_id: 1, body: $b })", { b: octets }));
            assert.equal(rejection.code, "yydb.unsupported");
        } finally {
            await database.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("`putFile` publishes an empty payload with a full-length digest", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-file-empty-"));
    try {
        const database = await Database.open(join(directory, "empty.yydx"));
        try {
            const file = await database.putFile(new Uint8Array());
            assert.equal(file.$yydb, "file");
            assert.equal(file.octets, 0);
            assert.match(file.digest, /^[0-9a-f]{64}$/);
            const read = await database.readFile(file);
            assert.equal(read.length, 0);
        } finally {
            await database.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("file windows cross chunks, clamp at EOF and reject negative offsets", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-file-window-"));
    try {
        const database = await Database.open(join(directory, "window.yydx"));
        try {
            const octets = Uint8Array.from({ length: 600 }, (_, i) => i % 251);
            const file = await database.putFile(octets, { chunkSize: 256 });
            assert.equal(file.octets, 600);

            const crossing = await database.readFileWindow(file, 250, 10);
            assert.deepEqual(Array.from(crossing), Array.from(octets.slice(250, 260)));

            const zero = await database.readFileWindow(file, 10, 0);
            assert.equal(zero.length, 0);

            const atEnd = await database.readFileWindow(file, 600, 10);
            assert.equal(atEnd.length, 0);

            const clamped = await database.readFileWindow(file, 595, 100);
            assert.deepEqual(Array.from(clamped), Array.from(octets.slice(595)));

            await assert.rejects(database.readFileWindow(file, -1, 10), /offset must be non-negative/);
        } finally {
            await database.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("string payloads and metadata fields are accepted without touching identity", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-file-meta-"));
    try {
        const database = await Database.open(join(directory, "meta.yydx"));
        try {
            const file = await database.putFile("hello world", { name: "greeting.txt", mediaType: "text/plain" });
            assert.equal(file.octets, 11);
            assert.equal(file.name, "greeting.txt");
            assert.equal(file.mediaType, "text/plain");
            const read = await database.readFile(file);
            assert.equal(new TextDecoder().decode(read), "hello world");
        } finally {
            await database.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("a tampered chunk fails closed with a digest violation after reopen", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-file-tamper-"));
    const objects = join(directory, "tamper-objects");
    try {
        const database = await Database.open(join(directory, "tamper.yydx"));
        const octets = Uint8Array.from({ length: 600 }, (_, i) => i % 251);
        const file = await database.putFile(octets, { chunkSize: 256 });
        await database.close();

        const walk = (dir: string): string[] =>
            readdirSync(dir, { withFileTypes: true }).flatMap((entry) =>
                entry.isDirectory() ? walk(join(dir, entry.name)) : [join(dir, entry.name)],
            );
        const blobs = walk(objects);
        const dataBlob = blobs.find((blob) => !blob.includes(file.manifest.hash));
        assert.ok(dataBlob, "a chunk object must exist next to the manifest");
        const original = readFileSync(dataBlob);
        const flipped = Uint8Array.from(original);
        flipped[Math.floor(original.length / 2)] ^= 0x01;
        writeFileSync(dataBlob, flipped);

        const reopened = await Database.open(join(directory, "tamper.yydx"));
        try {
            const rejection = await rejectDiagnostic(() => reopened.readFile(file));
            assert.equal(rejection.code, "yydb.object.corrupt");
            assert.match(rejection.message, /payload_hash mismatch/);
        } finally {
            await reopened.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("doctor, compaction and backup report the live layout", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-maintenance-"));
    try {
        const database = await Database.open(join(directory, "main.yydb"));
        await database.ensureSchema("table M {\n    @@m_id: i64,\n    note: utf8,\n}");
        await database.execute('M::insert({ m_id: 1, note: "kept" })');

        const report = await database.doctor();
        assert.equal(typeof report.last_commit_sequence, "number");
        assert.equal(typeof report.checkpoint_lag_records, "number");
        assert.ok(Array.isArray(report.issues));
        assert.ok(Array.isArray(report.namespaces));

        await database.backup(join(directory, "copy.yydb"));
        const copy = await Database.open(join(directory, "copy.yydb"));
        try {
            const rows = await copy.query<{ m_id: number; note: string }>("M.all().collect()");
            assert.deepEqual(rows, [{ m_id: 1, note: "kept" }]);
        } finally {
            await copy.close();
        }
        await database.close();

        const yydx = await Database.open(join(directory, "tree.yydx"));
        try {
            const compacted = await yydx.compactYydx();
            assert.equal(typeof compacted.checkpointed, "boolean");
            assert.equal(typeof compacted.orphans_reclaimed, "number");
        } finally {
            await yydx.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("`defineMicro` validates the Phase-1 contract before registration", () => {
    assert.throws(() => defineMicro({ name: " ", version: 1, args: [], returns: v.i64(), effect: "pure", deterministic: true, fn: () => 1n }), /micro name must not be empty/);
    assert.throws(() => defineMicro({ name: "ok", version: 0, args: [], returns: v.i64(), effect: "pure", deterministic: true, fn: () => 1n }), /version must be a positive integer/);
    assert.throws(() => defineMicro({ name: "ok", version: 1, args: [], returns: v.i64(), effect: "read", deterministic: true, fn: () => 1n }), /effect must be pure/);
    assert.throws(() => defineMicro({ name: "ok", version: 1, args: [], returns: v.i64(), effect: "pure", deterministic: false, fn: () => 1n }), /must be deterministic/);
    assert.throws(() => defineMicro({ name: "ok", version: 1, args: [v.f64()], returns: v.i64(), effect: "pure", deterministic: true, fn: () => 1n }), /outside the Phase-1 micro subset/);
    assert.throws(() => defineMicro({ name: "ok", version: 1, args: [], returns: v.f64(), effect: "pure", deterministic: true, fn: () => 1.5 }), /outside the Phase-1 micro subset/);

    const micro = defineMicro({
        name: "demo.double",
        version: 1,
        args: [v.i64()],
        returns: v.i64(),
        effect: "pure",
        deterministic: true,
        fn: (x) => x * 2n,
    });
    assert.equal(micro.mode, "scalar");
    assert.equal(micro.placement, "host");
});

test("`registerMicro` returns a session handle and refuses duplicates", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-micro-"));
    try {
        const database = await Database.open(join(directory, "micro.yydb"));
        try {
            const micro = defineMicro({
                name: "demo.identity",
                version: 1,
                args: [v.utf8()],
                returns: v.utf8(),
                effect: "pure",
                deterministic: true,
                fn: (x) => x,
            });
            const handle = await database.registerMicro(micro);
            assert.equal(handle.name, "demo.identity");
            assert.equal(handle.version, 1);
            assert.equal(typeof handle.hostId, "number");
            assert.equal(typeof handle.functionId, "string");
            assert.equal(handle.implementationVersion, 1);

            await assert.rejects(database.registerMicro(micro), /micro already registered/);
        } finally {
            await database.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("`callScalar` reports stable udf codes and invokes the installed host micro", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-micro-call-"));
    try {
        const database = await Database.open(join(directory, "call.yydb"));
        try {
            const micro = defineMicro({
                name: "demo.double",
                version: 1,
                args: [v.i64()],
                returns: v.i64(),
                effect: "pure",
                deterministic: true,
                fn: (x) => x * 2n,
            });
            await database.registerMicro(micro);

            const notFound = await rejectDiagnostic(() => database.callScalar("nope", 1, []));
            assert.equal(notFound.code, "yydb.udf.not_found");

            const wrongVersion = await rejectDiagnostic(() => database.callScalar("demo.double", 99, [1]));
            assert.equal(wrongVersion.code, "yydb.udf.version_mismatch");
            assert.equal(wrongVersion.params?.expected, "1");
            assert.equal(wrongVersion.params?.got, "99");

            const wrongArity = await rejectDiagnostic(() => database.callScalar("demo.double", 1, []));
            assert.equal(wrongArity.code, "yydb.udf.arity");

            assert.equal(await database.callScalar("demo.double", 1, [21]), 42);

            const shout = defineMicro({
                name: "demo.shout",
                version: 1,
                args: [v.utf8()],
                returns: v.utf8(),
                effect: "pure",
                deterministic: true,
                fn: (x) => `${x.toUpperCase()}!`,
            });
            await database.registerMicro(shout);
            assert.equal(await database.callScalar("demo.shout", 1, ["yydb"]), "YYDB!");
        } finally {
            await database.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});
