// Same-workload KV comparison: this repository's embedded engine against the
// real SQLite engine through `better-sqlite3` (the copy already installed for
// the workspace fixtures package).
//
// Both sides store the same keys and the same 64-byte values, load the same
// number of rows with the same page size and the same transaction boundary
// (KEYS_PER_TXN keys per commit), and read through the same point-get, range
// scan and reopen workloads.
//
// Run: node --experimental-strip-types bench/bench.ts

import { createRequire } from "node:module";
import { mkdtempSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { Database } from "@yydb/yydb";

const requireFromFixtures = createRequire(
  new URL("../../sqlite-fixtures/package.json", import.meta.url),
);
const BetterSqlite = requireFromFixtures("better-sqlite3") as new (
  path: string,
  options?: Record<string, unknown>,
) => {
  pragma(source: string): unknown;
  exec(source: string): void;
  prepare(source: string): {
    run(...args: unknown[]): unknown;
    all(...args: unknown[]): unknown[];
    get(...args: unknown[]): unknown;
  };
  close(): void;
};

const ROWS = Number(process.env.BENCH_ROWS ?? 20_000);
const KEYS_PER_TXN = Number(process.env.BENCH_TXN ?? 1_000);
const POINT_OPS = Number(process.env.BENCH_POINT ?? 10_000);
const RANGE_SCANS = Number(process.env.BENCH_RANGE ?? 200);
const RANGE_LEN = Number(process.env.BENCH_RANGE_LEN ?? 100);
const PAGE_SIZE = Number(process.env.BENCH_PAGE ?? 4_096);
const PAYLOAD = new Uint8Array(64).fill(0x78);

interface Timing {
  readonly name: string;
  readonly unit: string;
  readonly samples: number[];
}

function median(values: number[]): number {
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0 ? (sorted[middle - 1] + sorted[middle]) / 2 : sorted[middle];
}

function p95(values: number[]): number {
  const sorted = [...values].sort((a, b) => a - b);
  const index = Math.min(sorted.length - 1, Math.ceil(sorted.length * 0.95) - 1);
  return sorted[index];
}

function elapsedMs(start: bigint): number {
  return Number(process.hrtime.bigint() - start) / 1_000_000;
}

function key(row: number): string {
  return `row:${String(row).padStart(5, "0")}`;
}

function workspace(label: string): { path: string; dispose: () => void } {
  const root = mkdtempSync(join(tmpdir(), `yydb-bench-${label}-`));
  return {
    path: join(root, "main.yydb"),
    dispose: () => {
      try {
        rmSync(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
      } catch {
        // Windows may keep the handle briefly after close; temp dir is disposable.
      }
    },
  };
}

async function ourLoad(path: string): Promise<{ totalMs: number; reopenMs: number[] }> {
  const database = await Database.open(path);
  const start = process.hrtime.bigint();
  for (let offset = 0; offset < ROWS; offset += KEYS_PER_TXN) {
    await database.transaction(async (tx) => {
      const changes = [];
      for (let row = offset + 1; row <= offset + KEYS_PER_TXN; row += 1) {
        changes.push({ key: key(row), value: PAYLOAD });
      }
      await tx.batch(changes);
    });
  }
  const totalMs = elapsedMs(start);
  await database.close();

  const reopenMs: number[] = [];
  for (let index = 0; index < 20; index += 1) {
    const at = process.hrtime.bigint();
    const reopened = await Database.open(path);
    reopened.get(key(1));
    reopenMs.push(elapsedMs(at));
    await reopened.close();
  }
  return { totalMs, reopenMs };
}

async function ourPoint(path: string): Promise<number[]> {
  const database = await Database.open(path);
  const samples: number[] = [];
  for (let index = 0; index < POINT_OPS; index += 1) {
    const id = ((index * 7919) % ROWS) + 1;
    const start = process.hrtime.bigint();
    await database.get(key(id));
    samples.push(elapsedMs(start));
  }
  await database.close();
  return samples;
}

async function ourScan(path: string): Promise<number[]> {
  const database = await Database.open(path);
  const samples: number[] = [];
  let cursor: string | null = null;
  for (let index = 0; index < RANGE_SCANS; index += 1) {
    const start = process.hrtime.bigint();
    const rows = await database.scan({ prefix: "row:", after: cursor, limit: RANGE_LEN });
    if (rows.length !== RANGE_LEN) throw new Error(`${rows.length} scan rows, expected ${RANGE_LEN}`);
    cursor = index % 2 === 0 ? rows[rows.length - 1].key : null;
    samples.push(elapsedMs(start));
  }
  await database.close();
  return samples;
}

function oracleLoad(path: string, synchronous: "off" | "full"): number {
  const database = new BetterSqlite(path);
  database.pragma(`page_size = ${PAGE_SIZE}`);
  database.pragma(`synchronous = ${synchronous}`);
  database.pragma("journal_mode = memory");
  database.exec("create table kv (k text primary key, v blob)");
  const insert = database.prepare("insert into kv (k, v) values (?, ?)");
  const start = process.hrtime.bigint();
  for (let offset = 0; offset < ROWS; offset += KEYS_PER_TXN) {
    database.exec("begin");
    for (let row = offset + 1; row <= offset + KEYS_PER_TXN; row += 1) {
      insert.run(key(row), PAYLOAD);
    }
    database.exec("commit");
  }
  const totalMs = elapsedMs(start);
  database.close();
  return totalMs;
}

function oraclePoint(path: string): number[] {
  const database = new BetterSqlite(path);
  const lookup = database.prepare("select v from kv where k = ?");
  const samples: number[] = [];
  for (let index = 0; index < POINT_OPS; index += 1) {
    const id = ((index * 7919) % ROWS) + 1;
    const start = process.hrtime.bigint();
    lookup.get(key(id));
    samples.push(elapsedMs(start));
  }
  database.close();
  return samples;
}

function oracleScan(path: string): number[] {
  const database = new BetterSqlite(path);
  const range = database.prepare("select k, v from kv where k >= ? order by k limit ?");
  const samples: number[] = [];
  for (let index = 0; index < RANGE_SCANS; index += 1) {
    const start = process.hrtime.bigint();
    const rows = range.all(index % 2 === 0 ? "row:" : key(((index * 97) % ROWS) + 1), RANGE_LEN);
    if (rows.length !== RANGE_LEN) throw new Error(`${rows.length} scan rows, expected ${RANGE_LEN}`);
    samples.push(elapsedMs(start));
  }
  database.close();
  return samples;
}

function oracleReopen(path: string): number[] {
  const samples: number[] = [];
  for (let index = 0; index < 20; index += 1) {
    const start = process.hrtime.bigint();
    const database = new BetterSqlite(path);
    database.prepare("select v from kv where k = ?").get(key(1));
    database.close();
    samples.push(elapsedMs(start));
  }
  return samples;
}

function oracleSingle(path: string): number[] {
  const database = new BetterSqlite(path);
  database.pragma("synchronous = off");
  database.pragma("journal_mode = memory");
  database.exec("create table kv (k text primary key, v blob)");
  const insert = database.prepare("insert into kv (k, v) values (?, ?)");
  const samples: number[] = [];
  for (let row = 1; row <= 200; row += 1) {
    const start = process.hrtime.bigint();
    insert.run(key(row), PAYLOAD);
    samples.push(elapsedMs(start));
  }
  database.close();
  return samples;
}

async function ourSingle(path: string): Promise<number[]> {
  const database = await Database.open(path);
  const samples: number[] = [];
  for (let row = 1; row <= 200; row += 1) {
    const start = process.hrtime.bigint();
    await database.put(key(row), PAYLOAD);
    samples.push(elapsedMs(start));
  }
  await database.close();
  return samples;
}

const timings: Timing[] = [];
const files: Record<string, number> = {};
const ourSpace = workspace("ours");
const offSpace = workspace("sqlite-off");
const fullSpace = workspace("sqlite-full");
const singleOurSpace = workspace("single-ours");
const singleOffSpace = workspace("single-sqlite");

try {
  const load = await ourLoad(ourSpace.path);
  timings.push({
    name: "our batch load 20k keys (64B value, 1k/txn)",
    unit: "ms",
    samples: [load.totalMs],
  });
  for (const [label, target, mode] of [
    ["better-sqlite3 batch load 20k keys (4k page, 1k/txn, synchronous=off)", offSpace.path, "off"],
    ["better-sqlite3 batch load 20k keys (4k page, 1k/txn, synchronous=full)", fullSpace.path, "full"],
  ] as const) {
    timings.push({ name: label, unit: "ms", samples: [oracleLoad(target, mode)] });
  }

  timings.push({ name: "our single insert 200 (autocommit put)", unit: "us", samples: (await ourSingle(singleOurSpace.path)).map((ms) => ms * 1_000) });
  timings.push({ name: "better-sqlite3 single insert 200 (autocommit, synchronous=off)", unit: "us", samples: oracleSingle(singleOffSpace.path).map((ms) => ms * 1_000) });

  timings.push({ name: "our point get 10k", unit: "us", samples: (await ourPoint(ourSpace.path)).map((ms) => ms * 1_000) });
  timings.push({ name: "better-sqlite3 point lookup 10k (where k = ?)", unit: "us", samples: oraclePoint(offSpace.path).map((ms) => ms * 1_000) });

  timings.push({ name: `our range scan ${RANGE_SCANS}x${RANGE_LEN}`, unit: "us", samples: (await ourScan(ourSpace.path)).map((ms) => ms * 1_000) });
  timings.push({ name: `better-sqlite3 range scan ${RANGE_SCANS}x${RANGE_LEN}`, unit: "us", samples: oracleScan(offSpace.path).map((ms) => ms * 1_000) });

  timings.push({ name: "our reopen + get (20 rounds)", unit: "ms", samples: load.reopenMs });
  timings.push({ name: "better-sqlite3 reopen + get (20 rounds)", unit: "ms", samples: oracleReopen(offSpace.path) });

  files.our = statSync(ourSpace.path).size;
  files["better-sqlite3 off"] = statSync(offSpace.path).size;
  files["better-sqlite3 full"] = statSync(fullSpace.path).size;
} finally {
  for (const space of [ourSpace, offSpace, fullSpace, singleOurSpace, singleOffSpace]) space.dispose();
}

const lines: string[] = [];
lines.push(`yydb bench · rows=${ROWS} · txn=${KEYS_PER_TXN} · point=${POINT_OPS} · range=${RANGE_SCANS}x${RANGE_LEN} · page=${PAGE_SIZE}`);
lines.push("");
lines.push("| workload | median | p95 | unit |");
lines.push("| --- | --- | --- | --- |");
for (const timing of timings) {
  lines.push(`| ${timing.name} | ${median(timing.samples).toFixed(3)} | ${p95(timing.samples).toFixed(3)} | ${timing.unit} |`);
}
lines.push("");
lines.push("| file | bytes |");
lines.push("| --- | --- |");
for (const [name, bytes] of Object.entries(files)) lines.push(`| ${name} | ${bytes} |`);
lines.push("");
lines.push(
  "Notes: YYDB exposes no fsync or page-size switch, so its load is transacted writes followed by close. `synchronous=off` is the closest SQLite setting; `synchronous=full` is the durability reference. Range scans use the same key prefix and the same 100-row window.",
);
process.stdout.write(`${lines.join("\n")}\n`);
