import type { SqliteDecodedRow, SqliteRecordValue } from "@yydb/sqlite";

export const MAGIC = Buffer.from("SQLite format 3\0", "latin1");
export const PAGE_SIZE = 4096;
export const READ_LIMITS = { maxPages: 64, maxRows: 32 } as const;

export const SAMPLES_DDL =
    "CREATE TABLE samples (id INTEGER PRIMARY KEY, quantity INTEGER, score REAL, label TEXT, payload BLOB, note TEXT)";

export const CASE_DDL = 'CREATE TABLE "案例" (id INTEGER PRIMARY KEY, label TEXT)';

/** Canonical row payloads captured from `node:sqlite` for cross-checking YYDS-built files. */
export const SAMPLES_ROW_PAYLOADS: Readonly<Record<number, Buffer>> = {
    1: Buffer.from("070001071712002a400c000000000000616c70686100ff10", "hex"),
    2: Buffer.from("07000107150e19f9bfd000000000000062657461ff7365636f6e64", "hex"),
    3: Buffer.from("07000107170e17633ff400000000000067616d6d61ab6164646564", "hex"),
};

export interface TempFixture {
    directory: string;
    path: string;
}

export function columnText(column: SqliteRecordValue): string {
    assertKind(column, "text");
    return column.valueText ?? "";
}

export function columnInteger(column: SqliteRecordValue): bigint {
    assertKind(column, "integer");
    return column.valueInteger ?? 0n;
}

export function columnBlob(column: SqliteRecordValue): Buffer {
    assertKind(column, "blob");
    return column.valueBlob ?? Buffer.alloc(0);
}

export function rowById(rows: SqliteDecodedRow[], rowid: bigint): SqliteDecodedRow {
    const row = rows.find((candidate) => candidate.rowid === rowid);
    if (!row) {
        throw new Error(`rowid ${rowid} not found`);
    }
    return row;
}

function assertKind(column: SqliteRecordValue, kind: string): void {
    if (column.kind !== kind) {
        throw new Error(`expected ${kind} column, got ${column.kind}`);
    }
}
