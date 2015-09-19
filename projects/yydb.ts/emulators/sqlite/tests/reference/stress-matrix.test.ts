import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { SqliteWriter } from "@yydb/sqlite";
import { openBetterSqlite3, readWithPython, referencePython } from "@yydb/sqlite-fixtures";

const SKIP_NO_PYTHON = referencePython() ? false : "no python3/python with sqlite3 on this host";

/** Peak-memory sampler; samples must be sprinkled through the workload. */
class MemoryProbe {
    peakRss = 0;
    peakHeap = 0;

    sample(): void {
        const usage = process.memoryUsage();
        if (usage.rss > this.peakRss) this.peakRss = usage.rss;
        if (usage.heapUsed > this.peakHeap) this.peakHeap = usage.heapUsed;
    }
}

const mib = (bytes: number): string => (bytes / 1024 / 1024).toFixed(1);

/** Deterministic 32-bit LCG; same seed reproduces the same stream. */
function lcg(seed: number): () => number {
    let state = seed >>> 0;
    return () => {
        state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
        return state / 0x1_0000_0000;
    };
}

/** Deterministic Fisher-Yates shuffle of `1..=size`. */
function shuffledIds(size: number, seed: number): number[] {
    const ids = Array.from({ length: size }, (_unused, index) => index + 1);
    const random = lcg(seed);
    for (let index = ids.length - 1; index > 0; index -= 1) {
        const swap = Math.floor(random() * (index + 1));
        [ids[index], ids[swap]] = [ids[swap], ids[index]];
    }
    return ids;
}

/** Deterministic per-row values; generation and verification share these. */
function textAt(id: number): string {
    return `key-${id.toString().padStart(10, "0")}`;
}

function intAt(id: number): bigint {
    return BigInt((id * 31) % 100003);
}

function blobAt(id: number, length: number): Buffer {
    const bytes = Buffer.alloc(length);
    for (let offset = 0; offset < length; offset += 1) {
        bytes[offset] = (id * 31 + offset) % 251;
    }
    return bytes;
}

function sampleIds(count: number): number[] {
    const ids = [1, 2, 3, count - 2, count - 1, count].filter((id) => id >= 1 && id <= count);
    const step = Math.max(1, Math.floor(count / 97));
    for (let id = step; id <= count; id += step) ids.push(id);
    return [...new Set(ids)].sort((left, right) => left - right);
}

interface VerifyHandle {
    prepare(source: string): {
        run(...params: unknown[]): unknown;
        get(...params: unknown[]): unknown;
        all(...params: unknown[]): unknown[];
    };
    pragma(source: string, options?: { simple?: boolean }): unknown;
    defaultSafeIntegers(enabled: boolean): unknown;
    close(): void;
}

async function openVerifier(path: string): Promise<VerifyHandle> {
    const handle = (await openBetterSqlite3(path)) as VerifyHandle;
    handle.defaultSafeIntegers(true);
    return handle;
}

function assertIntegrity(db: VerifyHandle, label: string): void {
    const integrity = db.pragma("integrity_check", { simple: true });
    assert.equal(integrity, "ok", `${label}: PRAGMA integrity_check`);
}

function rowCount(db: VerifyHandle, table: string): number {
    const row = db.prepare(`SELECT COUNT(*) AS n FROM "${table}"`).get() as { n: bigint };
    return Number(row.n);
}

/** Verifies sampled rows of a `items(id, k, v, payload)` table against the generators. */
function assertSampledItems(db: VerifyHandle, scale: number, survivor: (id: number) => boolean, label: string): void {
    const statement = db.prepare("SELECT id, k, v, payload FROM items WHERE id = ?");
    let checked = 0;
    for (const id of sampleIds(scale)) {
        const row = statement.get(BigInt(id)) as { id: bigint; k: string; v: bigint; payload: Buffer } | undefined;
        if (!survivor(id)) {
            assert.equal(row, undefined, `${label}: deleted id ${id} must stay deleted`);
            continue;
        }
        assert.ok(row, `${label}: row id ${id} exists`);
        assert.equal(row.k, textAt(id), `${label}: id ${id} column k`);
        assert.equal(row.v, intAt(id), `${label}: id ${id} column v`);
        assert.ok(row.payload.equals(blobAt(id, 64)), `${label}: id ${id} payload bytes`);
        checked += 1;
    }
    assert.ok(checked > 0, `${label}: sample set is not empty`);
}

function createItems(writer: SqliteWriter): void {
    writer.createTable("items", [
        { name: "id", declaredType: "INTEGER", primaryKey: true },
        { name: "k", declaredType: "TEXT" },
        { name: "v", declaredType: "INTEGER" },
        { name: "payload", declaredType: "BLOB" },
    ]);
}

function insertItem(writer: SqliteWriter, id: number): void {
    writer.insert("items", [id, textAt(id), intAt(id), blobAt(id, 64)]);
}

function runInsertLoad(
    order: number[],
    transactional: boolean,
): { writer: SqliteWriter; probe: MemoryProbe; durationMs: number } {
    const probe = new MemoryProbe();
    const writer = SqliteWriter.create(4096);
    createItems(writer);
    if (transactional) writer.begin();
    const started = performance.now();
    const chunk = Math.max(1, Math.floor(order.length / 64));
    for (let offset = 0; offset < order.length; offset += 1) {
        insertItem(writer, order[offset]);
        if (offset % chunk === 0) probe.sample();
    }
    if (transactional) writer.commit();
    const durationMs = performance.now() - started;
    probe.sample();
    return { writer, probe, durationMs };
}

interface ScaleRun {
    scenario: string;
    scale: number;
    durationMs: number;
    probe: MemoryProbe;
    verify: (label: string) => Promise<void> | void;
    cleanup: () => void;
}

function report(t: test.TestContext, run: ScaleRun, note: string): void {
    const line =
        `${run.scenario} rows=${run.scale} load=${run.durationMs.toFixed(0)}ms ` +
        `peak-rss=${mib(run.probe.peakRss)}MiB peak-heap=${mib(run.probe.peakHeap)}MiB ${note}`;
    t.diagnostic(line);
    console.log(`[stress-matrix] ${line}`);
}

const SCALES = [10_000, 100_000, 500_000] as const;

function sequentialRun(scale: number): ScaleRun {
    const directory = mkdtempSync(join(tmpdir(), "yydb-stress-seq-"));
    const path = join(directory, "seq.sqlite");
    const { writer, probe, durationMs } = runInsertLoad(Array.from({ length: scale }, (_u, i) => i + 1), false);
    writer.save(path);
    probe.sample();
    return {
        scenario: "sequential",
        scale,
        durationMs,
        probe,
        verify: async (label) => {
            const db = await openVerifier(path);
            try {
                assertIntegrity(db, label);
                assert.equal(rowCount(db, "items"), scale, `${label}: row count`);
                assertSampledItems(db, scale, () => true, label);
            } finally {
                db.close();
            }
        },
        cleanup: () => rmSync(directory, { recursive: true, force: true }),
    };
}

function shuffledRun(scale: number): ScaleRun {
    const directory = mkdtempSync(join(tmpdir(), "yydb-stress-shuf-"));
    const path = join(directory, "shuf.sqlite");
    const { writer, probe, durationMs } = runInsertLoad(shuffledIds(scale, 0x5eed00 + scale), false);
    writer.save(path);
    probe.sample();
    return {
        scenario: "shuffled",
        scale,
        durationMs,
        probe,
        verify: async (label) => {
            const db = await openVerifier(path);
            try {
                assertIntegrity(db, label);
                assert.equal(rowCount(db, "items"), scale, `${label}: row count`);
                assertSampledItems(db, scale, () => true, label);
            } finally {
                db.close();
            }
        },
        cleanup: () => rmSync(directory, { recursive: true, force: true }),
    };
}

function bulkRun(scale: number): ScaleRun {
    const directory = mkdtempSync(join(tmpdir(), "yydb-stress-bulk-"));
    const path = join(directory, "bulk.sqlite");
    const { writer, probe, durationMs } = runInsertLoad(Array.from({ length: scale }, (_u, i) => i + 1), true);
    writer.save(path);
    probe.sample();
    return {
        scenario: "bulk-transaction",
        scale,
        durationMs,
        probe,
        verify: async (label) => {
            const db = await openVerifier(path);
            try {
                assertIntegrity(db, label);
                assert.equal(rowCount(db, "items"), scale, `${label}: row count`);
                assertSampledItems(db, scale, () => true, label);
            } finally {
                db.close();
            }
        },
        cleanup: () => rmSync(directory, { recursive: true, force: true }),
    };
}

function hollowRun(scale: number): ScaleRun {
    const directory = mkdtempSync(join(tmpdir(), "yydb-stress-hollow-"));
    const path = join(directory, "hollow.sqlite");
    // Scattered deletes punch freeblocks into many pages; the contiguous tail
    // delete drains whole pages so they must reach the freelist.
    const tailCut = Math.floor(scale * 0.95);
    const survivor = (id: number): boolean => id % 7 !== 1 && id % 7 !== 4 && id <= tailCut;
    const { writer, probe, durationMs } = runInsertLoad(Array.from({ length: scale }, (_u, i) => i + 1), false);
    const deleteStarted = performance.now();
    let deleted = 0;
    for (let id = 1; id <= scale; id += 1) {
        if (!survivor(id)) {
            assert.equal(writer.delete("items", id), true, `delete existing id ${id}`);
            deleted += 1;
        }
        if (id % 4096 === 0) probe.sample();
    }
    const deleteMs = performance.now() - deleteStarted;
    writer.save(path);
    probe.sample();
    return {
        scenario: "hollow-reopen",
        scale,
        durationMs: durationMs + deleteMs,
        probe,
        verify: async (label) => {
            const db = await openVerifier(path);
            try {
                assertIntegrity(db, `${label} first-save`);
                const expected = scale - deleted;
                assert.equal(rowCount(db, "items"), expected, `${label}: survivor count`);
                assert.ok(Number(db.pragma("freelist_count", { simple: true })) > 0, `${label}: emptied pages reach the freelist`);
                assertSampledItems(db, scale, survivor, `${label} first-save`);
            } finally {
                db.close();
            }
            // Reopen through our own writer, append into the hollowed file and
            // verify the result externally again.
            const reopened = SqliteWriter.open(path);
            for (let id = scale + 1; id <= scale + 1000; id += 1) {
                insertItem(reopened, id);
            }
            reopened.save(path);
            const after = await openVerifier(path);
            try {
                assertIntegrity(after, `${label} reopen-append`);
                assert.equal(rowCount(after, "items"), scale - deleted + 1000, `${label}: count after reopen append`);
                assertSampledItems(after, scale, survivor, `${label} reopen-append`);
                const statement = after.prepare("SELECT k, v, payload FROM items WHERE id = ?");
                for (const id of [scale + 1, scale + 500, scale + 1000]) {
                    const row = statement.get(BigInt(id)) as { k: string; v: bigint; payload: Buffer };
                    assert.equal(row.k, textAt(id), `${label}: appended id ${id} column k`);
                    assert.equal(row.v, intAt(id), `${label}: appended id ${id} column v`);
                    assert.ok(row.payload.equals(blobAt(id, 64)), `${label}: appended id ${id} payload`);
                }
            } finally {
                after.close();
            }
        },
        cleanup: () => rmSync(directory, { recursive: true, force: true }),
    };
}

function overflowRun(scale: number): ScaleRun {
    const directory = mkdtempSync(join(tmpdir(), "yydb-stress-ovf-"));
    const path = join(directory, "overflow.sqlite");
    // Keep the in-memory image bounded at the largest scale: small payloads
    // shrink with scale while the overflow share stays deterministic.
    const smallBytes = scale >= 500_000 ? 96 : 256;
    const overflowEvery = scale >= 500_000 ? 200 : 50;
    const overflowBytes = scale >= 500_000 ? 8_192 : 24_000;
    const isOverflow = (id: number): boolean => id % overflowEvery === 0;
    const payloadFor = (id: number): Buffer => blobAt(id, isOverflow(id) ? overflowBytes : smallBytes);

    const probe = new MemoryProbe();
    const writer = SqliteWriter.create(4096);
    writer.createTable("items", [
        { name: "id", declaredType: "INTEGER", primaryKey: true },
        { name: "kind", declaredType: "TEXT" },
        { name: "data", declaredType: "BLOB" },
    ]);
    const started = performance.now();
    const chunk = Math.max(1, Math.floor(scale / 64));
    for (let id = 1; id <= scale; id += 1) {
        writer.insert("items", [id, isOverflow(id) ? "overflow" : "inline", payloadFor(id)]);
        if (id % chunk === 0) probe.sample();
    }
    const durationMs = performance.now() - started;
    writer.save(path);
    probe.sample();
    return {
        scenario: "overflow-mixed",
        scale,
        durationMs,
        probe,
        verify: async (label) => {
            const db = await openVerifier(path);
            try {
                assertIntegrity(db, label);
                assert.equal(rowCount(db, "items"), scale, `${label}: row count`);
                const statement = db.prepare("SELECT kind, data FROM items WHERE id = ?");
                let overflowChecked = 0;
                for (const id of sampleIds(scale)) {
                    const row = statement.get(BigInt(id)) as { kind: string; data: Buffer };
                    const wantKind = isOverflow(id) ? "overflow" : "inline";
                    assert.equal(row.kind, wantKind, `${label}: id ${id} kind`);
                    const want = payloadFor(id);
                    assert.equal(row.data.length, want.length, `${label}: id ${id} payload length`);
                    assert.ok(row.data.equals(want), `${label}: id ${id} payload bytes`);
                    if (isOverflow(id)) overflowChecked += 1;
                }
                assert.ok(overflowChecked > 0, `${label}: overflow rows were sampled`);
            } finally {
                db.close();
            }
        },
        cleanup: () => rmSync(directory, { recursive: true, force: true }),
    };
}

function indexDenseRun(scale: number): ScaleRun {
    const directory = mkdtempSync(join(tmpdir(), "yydb-stress-idx-"));
    const path = join(directory, "indexed.sqlite");
    const aAt = (id: number): bigint => BigInt((id * 7919) % 50021);
    const bAt = (id: number): string => `grp-${(id * 131) % 977}`;
    const cAt = (id: number): number => ((id * 37) % 1000) / 4;

    const probe = new MemoryProbe();
    const writer = SqliteWriter.create(4096);
    writer.createTable("items", [
        { name: "id", declaredType: "INTEGER", primaryKey: true },
        { name: "a", declaredType: "INTEGER" },
        { name: "b", declaredType: "TEXT" },
        { name: "c", declaredType: "REAL" },
    ]);
    writer.createIndex("items_a", "items", ["a"], false);
    writer.createIndex("items_b", "items", ["b"], false);
    writer.createIndex("items_c", "items", ["c"], false);

    const started = performance.now();
    const chunk = Math.max(1, Math.floor(scale / 64));
    for (let id = 1; id <= scale; id += 1) {
        writer.insert("items", [id, aAt(id), bAt(id), cAt(id)]);
        if (id % chunk === 0) probe.sample();
    }
    // Updates and deletes churn the secondary indexes, not only the table tree.
    const updates = Math.max(10, Math.floor(scale / 20));
    const updatedIds = new Set<number>();
    for (let n = 0; n < updates; n += 1) {
        const id = (n * 613 % scale) + 1;
        writer.update("items", id, [{ keep: true }, { set: aAt(id) + 1n }, { keep: true }, { keep: true }]);
        updatedIds.add(id);
    }
    const aAfter = (id: number): bigint => (updatedIds.has(id) ? aAt(id) + 1n : aAt(id));
    let deleted = 0;
    const removed = new Set<number>();
    for (let n = 0; n < updates; n += 1) {
        const id = (n * 1777 % scale) + 1;
        if (writer.delete("items", id)) deleted += 1;
        removed.add(id);
    }
    const durationMs = performance.now() - started;
    writer.save(path);
    probe.sample();
    return {
        scenario: "index-dense",
        scale,
        durationMs,
        probe,
        verify: async (label) => {
            const db = await openVerifier(path);
            try {
                // integrity_check cross-verifies every index entry against the table.
                assertIntegrity(db, label);
                assert.equal(rowCount(db, "items"), scale - deleted, `${label}: survivor count`);
                const byId = db.prepare("SELECT a, b, c FROM items WHERE id = ?");
                const byA = db.prepare("SELECT id FROM items WHERE a = ? ORDER BY id");
                let indexProbes = 0;
                for (const id of sampleIds(scale)) {
                    if (removed.has(id)) {
                        assert.equal(byId.get(BigInt(id)), undefined, `${label}: deleted id ${id} stays deleted`);
                        continue;
                    }
                    const row = byId.get(BigInt(id)) as { a: bigint; b: string; c: number };
                    assert.equal(row.a, aAfter(id), `${label}: id ${id} column a`);
                    assert.equal(row.b, bAt(id), `${label}: id ${id} column b`);
                    assert.equal(row.c, cAt(id), `${label}: id ${id} column c`);
                    const viaIndex = (byA.all(row.a) as Array<{ id: bigint }>).map((entry) => Number(entry.id));
                    assert.ok(viaIndex.includes(id), `${label}: id ${id} reachable through index items_a`);
                    indexProbes += 1;
                }
                assert.ok(indexProbes > 0, `${label}: index probes ran`);
            } finally {
                db.close();
            }
        },
        cleanup: () => rmSync(directory, { recursive: true, force: true }),
    };
}

const SCENARIOS: Array<{ name: string; run: (scale: number) => ScaleRun }> = [
    { name: "sequential", run: sequentialRun },
    { name: "shuffled", run: shuffledRun },
    { name: "bulk-transaction", run: bulkRun },
    { name: "hollow-reopen", run: hollowRun },
    { name: "overflow-mixed", run: overflowRun },
    { name: "index-dense", run: indexDenseRun },
];

for (const scenario of SCENARIOS) {
    for (const scale of SCALES) {
        const timeout = scale >= 500_000 ? 1_200_000 : scale >= 100_000 ? 600_000 : 240_000;
        test(
            `stress ${scenario.name} at ${scale} rows stays externally valid`,
            { timeout },
            async (t) => {
                const run = scenario.run(scale);
                try {
                    await run.verify(`${scenario.name}@${scale}`);
                    report(t, run, "verification=ok");
                } finally {
                    run.cleanup();
                }
            },
        );
    }
}

test("stress sequential 10k also passes the Python sqlite3 oracle", { timeout: 240_000, skip: SKIP_NO_PYTHON }, (t) => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-stress-py-"));
    const path = join(directory, "seq.sqlite");
    try {
        // The same artifact is read by the Python oracle: integrity + sampled rows.
        const { writer, probe, durationMs } = runInsertLoad(Array.from({ length: 10_000 }, (_u, i) => i + 1), false);
        writer.save(path);
        probe.sample();
        const reportPy = readWithPython(path);
        assert.ok(reportPy, "python reader must run when python is available");
        assert.equal(reportPy.integrity, "ok", "python PRAGMA integrity_check");
        const rows = reportPy.tables.items;
        assert.equal(rows.length, 10_000, "python row count");
        for (const id of sampleIds(10_000)) {
            const row = rows[id - 1];
            assert.equal(row.rowid, BigInt(id), `python rowid at position ${id}`);
            assert.deepEqual(row.values[1], { kind: "text", value: textAt(id) }, `python id ${id} column k`);
        }
        t.diagnostic(`python oracle cross-check rows=10000 load=${durationMs.toFixed(0)}ms ok`);
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});
