import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";

import type { SqliteDecodedRow } from "@yydb/sqlite";

import { CASE_DDL, SAMPLES_DDL, type TempFixture } from "./shared.ts";

export interface NodeSamplesRow {
    rowid: number;
    quantity: number;
    score: number;
    label: string;
    payload: Buffer;
    note: string | null;
}

export interface NodeCaseRow {
    rowid: number;
    label: string;
}

export function createNodeSamplesFixture(rows?: readonly NodeSamplesRow[]): TempFixture & { bytes: Buffer } {
    const directory = mkdtempSync(join(tmpdir(), "yydb-node-sqlite-"));
    const path = join(directory, "samples.sqlite");
    const database = new DatabaseSync(path);
    try {
        database.exec(SAMPLES_DDL);
        const seed = rows ?? [
            { rowid: 1, quantity: 42, score: 3.5, label: "alpha", payload: Buffer.from([0x00, 0xff, 0x10]), note: null },
            { rowid: 2, quantity: -7, score: -0.25, label: "beta", payload: Buffer.from([0xff]), note: "second" },
        ];
        const insert = database.prepare(
            "INSERT INTO samples (rowid, quantity, score, label, payload, note) VALUES (?, ?, ?, ?, ?, ?)",
        );
        for (const row of seed) {
            insert.run(row.rowid, row.quantity, row.score, row.label, row.payload, row.note);
        }
    } finally {
        database.close();
    }
    return { directory, path, bytes: readFileSync(path) };
}

export function createNodeCaseFixture(rows?: readonly NodeCaseRow[]): TempFixture & { bytes: Buffer } {
    const directory = mkdtempSync(join(tmpdir(), "yydb-node-case-"));
    const path = join(directory, "case.sqlite");
    const database = new DatabaseSync(path);
    try {
        database.exec(CASE_DDL);
        const seed = rows ?? [
            { rowid: 1, label: "第一" },
            { rowid: 2, label: "第二" },
        ];
        const insert = database.prepare('INSERT INTO "案例" (rowid, label) VALUES (?, ?)');
        for (const row of seed) {
            insert.run(row.rowid, row.label);
        }
    } finally {
        database.close();
    }
    return { directory, path, bytes: readFileSync(path) };
}

export function readSamplesWithNode(path: string): NodeSamplesRow[] {
    const database = new DatabaseSync(path, { readOnly: true });
    try {
        const rows = database
            .prepare("SELECT id AS rowid, quantity, score, label, payload, note FROM samples ORDER BY id")
            .all() as Array<{
                rowid: number;
                quantity: number;
                score: number;
                label: string;
                payload: Buffer;
                note: string | null;
            }>;
        return rows.map((row) => ({
            rowid: row.rowid,
            quantity: row.quantity,
            score: row.score,
            label: row.label,
            payload: Buffer.from(row.payload),
            note: row.note,
        }));
    } finally {
        database.close();
    }
}

export function readCaseWithNode(path: string): NodeCaseRow[] {
    const database = new DatabaseSync(path, { readOnly: true });
    try {
        const rows = database
            .prepare('SELECT id AS rowid, label FROM "案例" ORDER BY id')
            .all() as Array<{ rowid: number; label: string }>;
        return rows.map((row) => ({ rowid: row.rowid, label: row.label }));
    } finally {
        database.close();
    }
}

export function nodeInsertSample(path: string, row: NodeSamplesRow): void {
    const database = new DatabaseSync(path);
    try {
        database
            .prepare("INSERT INTO samples (rowid, quantity, score, label, payload, note) VALUES (?, ?, ?, ?, ?, ?)")
            .run(row.rowid, row.quantity, row.score, row.label, row.payload, row.note);
    } finally {
        database.close();
    }
}

export function nodeUpdateSampleLabel(path: string, rowid: number, label: string): void {
    const database = new DatabaseSync(path);
    try {
        database.prepare("UPDATE samples SET label = ? WHERE rowid = ?").run(label, rowid);
    } finally {
        database.close();
    }
}

export function nodeInsertCase(path: string, row: NodeCaseRow): void {
    const database = new DatabaseSync(path);
    try {
        database.prepare('INSERT INTO "案例" (rowid, label) VALUES (?, ?)').run(row.rowid, row.label);
    } finally {
        database.close();
    }
}

export function assertSamplesMatchYyds(rows: SqliteDecodedRow[], expected: readonly NodeSamplesRow[]): void {
    if (rows.length !== expected.length) {
        throw new Error(`expected ${expected.length} rows, got ${rows.length}`);
    }
    for (const [index, row] of rows.entries()) {
        const want = expected[index];
        if (row.rowid !== BigInt(want.rowid)) {
            throw new Error(`row ${index}: rowid ${row.rowid} != ${want.rowid}`);
        }
        const [, quantity, score, label, payload, note] = row.columns;
        if (quantity.valueInteger !== BigInt(want.quantity)) {
            throw new Error(`row ${index}: quantity mismatch`);
        }
        if (score.valueReal !== want.score) {
            throw new Error(`row ${index}: score mismatch`);
        }
        if (label.valueText !== want.label) {
            throw new Error(`row ${index}: label mismatch`);
        }
        if (!payload.valueBlob?.equals(want.payload)) {
            throw new Error(`row ${index}: payload mismatch`);
        }
        const noteText = note.kind === "null" ? null : note.valueText ?? null;
        if (noteText !== want.note) {
            throw new Error(`row ${index}: note mismatch`);
        }
    }
}
