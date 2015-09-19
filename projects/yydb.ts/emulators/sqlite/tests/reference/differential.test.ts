import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { SqliteWriter } from "@yydb/sqlite";
import { openBetterSqlite3, readWithPython, referencePython } from "@yydb/sqlite-fixtures";

const SKIP_NO_PYTHON = referencePython() ? false : "no python3/python with sqlite3 on this host";

/** Deterministic 32-bit LCG; the same seed replays the same operation stream. */
function lcg(seed: number): () => number {
    let state = seed >>> 0;
    return () => {
        state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
        return state / 0x1_0000_0000;
    };
}

type DiffOp =
    | { kind: "insert"; id: number; k: string | null; v: number | null; f: number | null; b: number[] | null }
    | { kind: "update"; id: number; k: string | null; v: number | null }
    | { kind: "delete"; id: number }
    | { kind: "begin" }
    | { kind: "commit" }
    | { kind: "rollback" }
    | { kind: "checkpoint" };

const OP_WEIGHTS: Array<[DiffOp["kind"], number]> = [
    ["insert", 45],
    ["update", 20],
    ["delete", 15],
    ["begin", 8],
    ["commit", 6],
    ["rollback", 4],
    ["checkpoint", 2],
];

/**
 * Yields the concrete operation stream for a seed. One generator definition
 * feeds every oracle, so implementations cannot drift apart.
 */
function* opStream(seed: number, count: number): Generator<DiffOp, void, void> {
    const random = lcg(seed);
    const live: number[] = [];
    let nextId = 1;
    let inTxn = false;
    let txnSnapshot: number[] | null = null;
    let emitted = 0;
    while (emitted < count) {
        const total = OP_WEIGHTS.reduce((sum, [, weight]) => sum + weight, 0);
        let ticket = Math.floor(random() * total);
        let kind: DiffOp["kind"] = "insert";
        for (const [name, weight] of OP_WEIGHTS) {
            ticket -= weight;
            if (ticket < 0) {
                kind = name;
                break;
            }
        }
        switch (kind) {
            case "insert": {
                const id = nextId;
                nextId += 1;
                const k = random() < 0.85 ? `k-${id}-${random() < 0.3 ? "中文" : "ascii"}-${Math.floor(random() * 97)}` : null;
                const v = random() < 0.8 ? Math.floor(random() * 50_000) : null;
                const f = random() < 0.6 ? Math.floor(random() * 10_000) / 4 : null;
                const size = random() < 0.15 ? 1 + Math.floor(random() * 9_000) : 1 + Math.floor(random() * 64);
                const b = random() < 0.7 ? Array.from({ length: size }, (_u, i) => (id * 31 + i) % 251) : null;
                live.push(id);
                emitted += 1;
                yield { kind: "insert", id, k, v, f, b };
                break;
            }
            case "update": {
                if (live.length === 0) continue;
                const id = live[Math.floor(random() * live.length)];
                const k = random() < 0.5 ? `upd-${id}-${Math.floor(random() * 31)}` : null;
                const v = random() < 0.5 ? Math.floor(random() * 50_000) : null;
                emitted += 1;
                yield { kind: "update", id, k, v };
                break;
            }
            case "delete": {
                if (live.length === 0) continue;
                const position = Math.floor(random() * live.length);
                const id = live[position];
                live.splice(position, 1);
                emitted += 1;
                yield { kind: "delete", id };
                break;
            }
            case "begin": {
                if (inTxn) continue;
                inTxn = true;
                txnSnapshot = [...live];
                emitted += 1;
                yield { kind: "begin" };
                break;
            }
            case "commit": {
                if (!inTxn) continue;
                inTxn = false;
                txnSnapshot = null;
                emitted += 1;
                yield { kind: "commit" };
                break;
            }
            case "rollback": {
                if (!inTxn) continue;
                inTxn = false;
                // Both engines restore the pre-transaction state; the model
                // must restore its live set the same way.
                live.length = 0;
                live.push(...(txnSnapshot ?? []));
                txnSnapshot = null;
                emitted += 1;
                yield { kind: "rollback" };
                break;
            }
            case "checkpoint": {
                emitted += 1;
                yield { kind: "checkpoint" };
                break;
            }
        }
    }
}

/** One row in the normalized comparison shape shared by every engine. */
interface NormalRow {
    rowid: bigint;
    k: string | null;
    v: bigint | null;
    f: number | null;
    b: Buffer | null;
}

interface Oracle {
    prepare(source: string): {
        run(...params: unknown[]): unknown;
        get(...params: unknown[]): unknown;
        all(...params: unknown[]): unknown[];
    };
    pragma(source: string, options?: { simple?: boolean }): unknown;
    defaultSafeIntegers(enabled: boolean): unknown;
    close(): void;
}

function normalizeOracleRows(rows: Array<{ __rowid: bigint; k: string | null; v: bigint | null; f: number | null; b: Buffer | null }>): NormalRow[] {
    return rows.map((row) => ({
        rowid: BigInt(row.__rowid),
        k: row.k,
        v: row.v === null ? null : BigInt(row.v),
        f: row.f,
        b: row.b === null ? null : Buffer.from(row.b),
    }));
}

function oursRows(ours: SqliteWriter): NormalRow[] {
    return ours.table("diff").map((row) => {
        const columns = row.columns;
        return {
            rowid: row.rowid,
            k: columns[1].kind === "text" ? (columns[1].valueText ?? null) : null,
            v: columns[2].kind === "integer" ? (columns[2].valueInteger ?? null) : null,
            f: columns[3].kind === "real" ? (columns[3].valueReal ?? null) : null,
            b: columns[4].kind === "blob" && columns[4].valueBlob ? Buffer.from(columns[4].valueBlob) : null,
        };
    });
}

function assertRowsEqual(actual: NormalRow[], expected: NormalRow[], label: string, history: string[]): void {
    assert.equal(actual.length, expected.length, `${label}: row count\n${history.join("\n")}`);
    for (const [position, want] of expected.entries()) {
        const got = actual[position];
        assert.equal(got.rowid, want.rowid, `${label}: rowid at position ${position}\n${history.join("\n")}`);
        assert.equal(got.k, want.k, `${label}: row ${want.rowid} column k\n${history.join("\n")}`);
        assert.equal(got.v, want.v, `${label}: row ${want.rowid} column v\n${history.join("\n")}`);
        assert.equal(got.f, want.f, `${label}: row ${want.rowid} column f\n${history.join("\n")}`);
        assert.deepEqual(got.b, want.b, `${label}: row ${want.rowid} column b\n${history.join("\n")}`);
    }
}

/** Runs the seeded stream over both engines, comparing row sets and index probes. */
async function differential(seed: number, operations: number): Promise<void> {
    const directory = mkdtempSync(join(tmpdir(), "yydb-diff-"));
    const ourPath = join(directory, "ours.sqlite");
    const oraclePath = join(directory, "oracle.sqlite");
    const history: string[] = [];
    const remember = (line: string): void => {
        history.push(line);
        if (history.length > 60) history.shift();
    };

    const ours = SqliteWriter.create(4096);
    ours.createTable("diff", [
        { name: "id", declaredType: "INTEGER", primaryKey: true },
        { name: "k", declaredType: "TEXT" },
        { name: "v", declaredType: "INTEGER" },
        { name: "f", declaredType: "REAL" },
        { name: "b", declaredType: "BLOB" },
    ]);
    ours.createIndex("diff_v", "diff", ["v"], false);
    ours.createIndex("diff_k", "diff", ["k"], false);

    const oracle = (await openBetterSqlite3(oraclePath)) as Oracle;
    oracle.defaultSafeIntegers(true);
    oracle.prepare("CREATE TABLE diff (id INTEGER PRIMARY KEY, k TEXT, v INTEGER, f REAL, b BLOB)").run();
    oracle.prepare("CREATE INDEX diff_v ON diff (v)").run();
    oracle.prepare("CREATE INDEX diff_k ON diff (k)").run();
    const insertOracle = oracle.prepare("INSERT INTO diff (id, k, v, f, b) VALUES (?, ?, ?, ?, ?)");
    const updateOracle = oracle.prepare("UPDATE diff SET k = ?, v = ? WHERE id = ?");
    const deleteOracle = oracle.prepare("DELETE FROM diff WHERE id = ?");

    const oracleRows = (): NormalRow[] =>
        normalizeOracleRows(oracle.prepare("SELECT rowid AS __rowid, k, v, f, b FROM diff ORDER BY rowid").all() as Array<{
            rowid: bigint;
            k: string | null;
            v: bigint | null;
            f: number | null;
            b: Buffer | null;
        }>);

    let opIndex = 0;
    try {
        for (const op of opStream(seed, operations)) {
            opIndex += 1;
            switch (op.kind) {
                case "insert": {
                    remember(`insert ${op.id}`);
                    ours.insert("diff", [op.id, op.k, op.v === null ? null : BigInt(op.v), op.f, op.b === null ? null : new Uint8Array(op.b)]);
                    insertOracle.run(BigInt(op.id), op.k, op.v === null ? null : BigInt(op.v), op.f, op.b === null ? null : Buffer.from(op.b));
                    break;
                }
                case "update": {
                    remember(`update ${op.id}`);
                    const changed = ours.update("diff", op.id, [
                        { keep: true },
                        { set: op.k },
                        { set: op.v === null ? null : BigInt(op.v) },
                        { keep: true },
                        { keep: true },
                    ]);
                    assert.equal(changed, true, `update of live id ${op.id} must apply\n${history.join("\n")}`);
                    updateOracle.run(op.k, op.v === null ? null : BigInt(op.v), BigInt(op.id));
                    break;
                }
                case "delete": {
                    remember(`delete ${op.id}`);
                    assert.equal(ours.delete("diff", op.id), true, `delete of live id ${op.id}\n${history.join("\n")}`);
                    deleteOracle.run(BigInt(op.id));
                    break;
                }
                case "begin":
                    remember("begin");
                    ours.begin();
                    oracle.prepare("BEGIN").run();
                    break;
                case "commit":
                    remember("commit");
                    ours.commit();
                    oracle.prepare("COMMIT").run();
                    break;
                case "rollback":
                    remember("rollback");
                    ours.rollback();
                    oracle.prepare("ROLLBACK").run();
                    break;
                case "checkpoint": {
                    const expected = oracleRows();
                    assertRowsEqual(oursRows(ours), expected, `checkpoint at op ${opIndex}`, history);
                    // Index-ordered probes through the oracle must agree with
                    // filtering its own row set; our saved file is probed after save.
                    for (const want of expected.slice(0, 3)) {
                        if (want.v !== null) {
                            const viaIndex = (oracle.prepare("SELECT id FROM diff WHERE v = ? ORDER BY id").all(want.v) as Array<{ id: bigint }>)
                                .map((row) => BigInt(row.id));
                            const viaRows = expected.filter((row) => row.v === want.v).map((row) => row.rowid);
                            assert.deepEqual(viaIndex, viaRows, `checkpoint ${opIndex}: oracle index diff_v probe`);
                        }
                        if (want.k !== null) {
                            const viaIndex = (oracle.prepare("SELECT id FROM diff WHERE k = ? ORDER BY id").all(want.k) as Array<{ id: bigint }>)
                                .map((row) => BigInt(row.id));
                            const viaRows = expected.filter((row) => row.k === want.k).map((row) => row.rowid);
                            assert.deepEqual(viaIndex, viaRows, `checkpoint ${opIndex}: oracle index diff_k probe`);
                        }
                    }
                    break;
                }
            }
        }
        assertRowsEqual(oursRows(ours), oracleRows(), "final state", history);

        // Persist our image and verify it externally: integrity, full rows and
        // index-driven lookups against the oracle's final state.
        ours.save(ourPath);
        const external = (await openBetterSqlite3(ourPath)) as Oracle;
        external.defaultSafeIntegers(true);
        try {
            assert.equal(external.pragma("integrity_check", { simple: true }), "ok", "saved file integrity_check");
            const persisted = normalizeOracleRows(
                external.prepare("SELECT rowid AS __rowid, k, v, f, b FROM diff ORDER BY rowid").all() as Array<{
                    rowid: bigint;
                    k: string | null;
                    v: bigint | null;
                    f: number | null;
                    b: Buffer | null;
                }>,
            );
            const expected = oracleRows();
            assertRowsEqual(persisted, expected, "saved file vs oracle final state", history);
            const probe = external.prepare("SELECT id FROM diff WHERE v = ? ORDER BY id");
            for (const want of expected.slice(0, 5)) {
                if (want.v === null) continue;
                const viaIndex = (probe.all(want.v) as Array<{ id: bigint }>).map((row) => BigInt(row.id));
                const viaRows = expected.filter((row) => row.v === want.v).map((row) => row.rowid);
                assert.deepEqual(viaIndex, viaRows, `saved file index diff_v probe for v=${want.v}`);
            }
        } finally {
            external.close();
        }
    } finally {
        oracle.close();
        rmSync(directory, { recursive: true, force: true });
    }
}

test("seeded operation stream converges with better-sqlite3 (seed 20261103)", { timeout: 600_000 }, async () => {
    await differential(20261103, 3_000);
});

test("seeded operation stream converges with better-sqlite3 (seed 424242)", { timeout: 600_000 }, async () => {
    await differential(424242, 3_000);
});

test("saved differential artifact also satisfies the Python sqlite3 oracle", { timeout: 600_000, skip: SKIP_NO_PYTHON }, async () => {
    const seed = 20261103;
    const operations = 3_000;
    const directory = mkdtempSync(join(tmpdir(), "yydb-diff-py-"));
    const path = join(directory, "ours.sqlite");
    try {
        // Replay the identical concrete stream into our writer and save it.
        const ops = [...opStream(seed, operations)];
        const writer = SqliteWriter.create(4096);
        writer.createTable("diff", [
            { name: "id", declaredType: "INTEGER", primaryKey: true },
            { name: "k", declaredType: "TEXT" },
            { name: "v", declaredType: "INTEGER" },
            { name: "f", declaredType: "REAL" },
            { name: "b", declaredType: "BLOB" },
        ]);
        writer.createIndex("diff_v", "diff", ["v"], false);
        writer.createIndex("diff_k", "diff", ["k"], false);
        for (const op of ops) {
            switch (op.kind) {
                case "insert":
                    writer.insert("diff", [op.id, op.k, op.v === null ? null : BigInt(op.v), op.f, op.b === null ? null : new Uint8Array(op.b)]);
                    break;
                case "update":
                    writer.update("diff", op.id, [{ keep: true }, { set: op.k }, { set: op.v === null ? null : BigInt(op.v) }, { keep: true }, { keep: true }]);
                    break;
                case "delete":
                    writer.delete("diff", op.id);
                    break;
                case "begin":
                    writer.begin();
                    break;
                case "commit":
                    writer.commit();
                    break;
                case "rollback":
                    writer.rollback();
                    break;
                case "checkpoint":
                    break;
            }
        }
        writer.save(path);

        // The Python oracle executes the same concrete operations on its own
        // sqlite3 and dumps the final row set; both must agree with our file.
        // The operation list travels as a file because it can be megabytes.
        const opsPath = join(directory, "ops.json");
        writeFileSync(opsPath, JSON.stringify(ops));
        const python = referencePython();
        assert.ok(python);
        const { spawnSync } = await import("node:child_process");
        const replay = spawnSync(
            python,
            [
                "-c",
                `import json, sqlite3, sys
with open(sys.argv[1], "r", encoding="utf-8") as handle:
    ops = json.load(handle)
db = sqlite3.connect(":memory:", isolation_level=None)
db.execute("CREATE TABLE diff (id INTEGER PRIMARY KEY, k TEXT, v INTEGER, f REAL, b BLOB)")
db.execute("CREATE INDEX diff_v ON diff (v)")
db.execute("CREATE INDEX diff_k ON diff (k)")
for op in ops:
    kind = op["kind"]
    if kind == "insert":
        blob = bytes(op["b"]) if op["b"] is not None else None
        db.execute("INSERT INTO diff (id,k,v,f,b) VALUES (?,?,?,?,?)", (op["id"], op["k"], op["v"], op["f"], blob))
    elif kind == "update":
        db.execute("UPDATE diff SET k=?, v=? WHERE id=?", (op["k"], op["v"], op["id"]))
    elif kind == "delete":
        db.execute("DELETE FROM diff WHERE id=?", (op["id"],))
    elif kind == "begin":
        db.execute("BEGIN")
    elif kind == "commit":
        db.execute("COMMIT")
    elif kind == "rollback":
        db.execute("ROLLBACK")
out = [[r[0], r[1], r[2], r[3], None if r[4] == "null" else r[5]] for r in db.execute("SELECT rowid, k, v, f, typeof(b), hex(b) FROM diff ORDER BY rowid")]
json.dump(out, sys.stdout, ensure_ascii=False)`,
                opsPath,
            ],
            { encoding: "utf8", timeout: 300_000, maxBuffer: 512 * 1024 * 1024 },
        );
        assert.equal(replay.status, 0, `spawn error: ${String(replay.error)} stderr: ${replay.stderr}`);
        const rawExpected = JSON.parse(replay.stdout) as Array<[number, string | null, number | null, number | null, string | null]>;
        // Python's `hex()` renders uppercase; our reader renders lowercase.
        const expected = rawExpected.map(([rowid, k, v, f, b]) => [rowid, k, v, f, b === null ? null : b.toLowerCase()]);

        const report = readWithPython(path);
        assert.ok(report, "python reader must run when python is available");
        assert.equal(report.integrity, "ok", "python PRAGMA integrity_check");
        const actual = report.tables.diff.map((row) => {
            const real = row.values[3];
            let realValue: number | null = null;
            if (real.kind === "real") {
                const bits = Buffer.alloc(8);
                bits.writeBigUInt64BE(real.bits);
                realValue = bits.readDoubleBE();
            }
            return [
                Number(row.rowid),
                row.values[1].kind === "text" ? row.values[1].value : null,
                row.values[2].kind === "integer" ? Number(row.values[2].value) : null,
                realValue,
                row.values[4].kind === "blob" ? row.values[4].value.toString("hex") : null,
            ];
        });
        assert.deepEqual(actual, expected, "python replay of the same operation stream matches the saved artifact");
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});
