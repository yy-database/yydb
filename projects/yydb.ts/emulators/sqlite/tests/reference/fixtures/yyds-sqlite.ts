import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { TempFixture } from "./shared.ts";
import { SAMPLES_ROW_PAYLOADS } from "./shared.ts";
import { buildCaseDatabase, buildSamplesDatabase, type CaseRow } from "./yyds-format.ts";

export function writeYydsSamplesFixture(
    rows: Array<{ rowid: number; payload?: Buffer }>,
    name = "samples.sqlite",
): TempFixture & { bytes: Buffer } {
    const directory = mkdtempSync(join(tmpdir(), "yydb-yyds-sqlite-"));
    const path = join(directory, name);
    const bytes = buildSamplesDatabase(
        rows.map((row) => ({
            rowid: row.rowid,
            payload: row.payload ?? SAMPLES_ROW_PAYLOADS[row.rowid],
        })),
    );
    writeFileSync(path, bytes);
    return { directory, path, bytes };
}

export function writeYydsCaseFixture(rows: readonly CaseRow[], name = "case.sqlite"): TempFixture & { bytes: Buffer } {
    const directory = mkdtempSync(join(tmpdir(), "yydb-yyds-case-"));
    const path = join(directory, name);
    const bytes = buildCaseDatabase(rows);
    writeFileSync(path, bytes);
    return { directory, path, bytes };
}

export function rewriteYydsCaseFixture(path: string, rows: readonly CaseRow[]): Buffer {
    const bytes = buildCaseDatabase(rows);
    writeFileSync(path, bytes);
    return bytes;
}

export function rewriteYydsSamplesFixture(
    path: string,
    rows: Array<{ rowid: number; payload?: Buffer }>,
): Buffer {
    const bytes = buildSamplesDatabase(
        rows.map((row) => ({
            rowid: row.rowid,
            payload: row.payload ?? SAMPLES_ROW_PAYLOADS[row.rowid],
        })),
    );
    writeFileSync(path, bytes);
    return bytes;
}
