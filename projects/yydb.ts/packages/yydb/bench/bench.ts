// Same-workload KV benchmark for the embedded engine.
//
// The workloads cover batch load, single-key autocommit writes, point gets,
// range scans and reopen + get. Only YYDB is measured.
//
// Run: node --experimental-strip-types bench/bench.ts

import { mkdtempSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { Database, loadNative } from "@yydb/yydb";

// Bench stats run in `database.harness` through the napi primitive
// `harnessBenchSummary` (middle-two median, nearest-rank p95); the formulas
// match the previous local implementations bit-for-bit.
const benchNative = loadNative() as {
  harnessBenchSummary: (samplesMs: number[]) => { medianMs: number; p95Ms: number };
};

const ROWS = Number(process.env.BENCH_ROWS ?? 20_000);
const KEYS_PER_TXN = Number(process.env.BENCH_TXN ?? 1_000);
const POINT_OPS = Number(process.env.BENCH_POINT ?? 10_000);
const RANGE_SCANS = Number(process.env.BENCH_RANGE ?? 200);
const RANGE_LEN = Number(process.env.BENCH_RANGE_LEN ?? 100);
const PAYLOAD = new Uint8Array(64).fill(0x78);

interface Timing {
  readonly name: string;
  readonly unit: string;
  readonly samples: number[];
}

function median(values: number[]): number {
  return benchNative.harnessBenchSummary(values).medianMs;
}

function p95(values: number[]): number {
  return benchNative.harnessBenchSummary(values).p95Ms;
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
const singleOurSpace = workspace("single-ours");

try {
  const load = await ourLoad(ourSpace.path);
  timings.push({
    name: "our batch load 20k keys (64B value, 1k/txn)",
    unit: "ms",
    samples: [load.totalMs],
  });

  timings.push({ name: "our single insert 200 (autocommit put)", unit: "us", samples: (await ourSingle(singleOurSpace.path)).map((ms) => ms * 1_000) });

  timings.push({ name: "our point get 10k", unit: "us", samples: (await ourPoint(ourSpace.path)).map((ms) => ms * 1_000) });

  timings.push({ name: `our range scan ${RANGE_SCANS}x${RANGE_LEN}`, unit: "us", samples: (await ourScan(ourSpace.path)).map((ms) => ms * 1_000) });

  timings.push({ name: "our reopen + get (20 rounds)", unit: "ms", samples: load.reopenMs });

  files.our = statSync(ourSpace.path).size;
} finally {
  for (const space of [ourSpace, singleOurSpace]) space.dispose();
}

const lines: string[] = [];
lines.push(`yydb bench · rows=${ROWS} · txn=${KEYS_PER_TXN} · point=${POINT_OPS} · range=${RANGE_SCANS}x${RANGE_LEN}`);
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
lines.push("Notes: YYDB exposes no fsync or page-size switch, so its load is transacted writes followed by close. Range scans use the same key prefix and the same 100-row window.");
process.stdout.write(`${lines.join("\n")}\n`);
