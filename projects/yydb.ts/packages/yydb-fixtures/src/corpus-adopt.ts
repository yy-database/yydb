import { SqliteDatabase } from "@yydb/sqlite";
import type { ReferenceRow, ReferenceValue } from "@yydb/sqlite-fixtures";

export const SIMPLE_SCHEMA = `table Simple {
    @@id: i64,
    name: utf8,
    note: utf8,
}`;

export const TYPES_SCHEMA = `table TypesRow {
    @@id: i64,
    txt: utf8,
    lead: utf8,
    r: f64,
    payload: bytes,
    n: i64,
    empty: utf8,
}`;

const READ_LIMITS = { maxPages: 256, maxRows: 64, maxPayloadBytes: 1_048_576 } as const;

/** One VOS write program plus the named parameters it binds. */
export interface AdoptionProgram {
    source: string;
    params?: Record<string, unknown>;
}

function realNumber(bits: bigint): number {
    const buffer = Buffer.alloc(8);
    buffer.writeBigUInt64BE(bits);
    return buffer.readDoubleBE();
}

/**
 * Renders one reference value as a VOS field initializer.
 *
 * VOS only accepts a literal field initializer or a bound parameter, and its
 * grammar has no negative number literal. Values that cannot be spelled exactly
 * as a literal (negative integers, every REAL, and `bytes`, which has no literal
 * form at all) are bound as named parameters so the host sends the exact value.
 */
function fieldExpr(field: string, value: ReferenceValue, params: Record<string, unknown>): string {
    switch (value.kind) {
        case "null":
            return "null";
        case "integer":
            if (value.value >= 0n) {
                return value.value.toString();
            }
            params[field] = value.value;
            return `$${field}`;
        case "real":
            params[field] = realNumber(value.bits);
            return `$${field}`;
        case "text":
            return JSON.stringify(value.value);
        case "blob":
            params[field] = new Uint8Array(value.value);
            return `$${field}`;
    }
}

function readTable(sqlitePath: string, table: string): ReferenceRow[] {
    const database = SqliteDatabase.open(sqlitePath);
    return database.table(table, READ_LIMITS).map((row) => ({
        rowid: Number(row.rowid),
        values: row.columns.map((column) => {
            if (column.kind === "null") {
                return { kind: "null" };
            }
            if (column.kind === "integer") {
                return { kind: "integer", value: column.valueInteger };
            }
            if (column.kind === "real") {
                const buffer = Buffer.alloc(8);
                buffer.writeDoubleBE(column.valueReal ?? 0);
                return { kind: "real", bits: buffer.readBigUInt64BE() };
            }
            if (column.kind === "text") {
                return { kind: "text", value: column.valueText ?? "" };
            }
            return { kind: "blob", value: column.valueBlob ?? Buffer.alloc(0) };
        }),
    }));
}

export function buildSimpleAdoptionPrograms(sqlitePath: string): AdoptionProgram[] {
    return readTable(sqlitePath, "simple").map((row) => {
        const [, name, note] = row.values;
        const params: Record<string, unknown> = {};
        const source = `Simple { id: ${row.rowid}, name: ${fieldExpr("name", name, params)}, note: ${fieldExpr("note", note, params)} }.insert()`;
        return Object.keys(params).length > 0 ? { source, params } : { source };
    });
}

export function buildTypesAdoptionPrograms(sqlitePath: string): AdoptionProgram[] {
    return readTable(sqlitePath, "types").map((row) => {
        const [, txt, lead, r, blob, n, empty] = row.values;
        if (blob.kind !== "blob") {
            throw new Error(`the reference types table row ${row.rowid} has no blob payload`);
        }
        const params: Record<string, unknown> = {};
        const fields = [
            `id: ${row.rowid}`,
            `txt: ${fieldExpr("txt", txt, params)}`,
            `lead: ${fieldExpr("lead", lead, params)}`,
            `r: ${fieldExpr("r", r, params)}`,
            `payload: ${fieldExpr("payload", blob, params)}`,
            `n: ${fieldExpr("n", n, params)}`,
            `empty: ${fieldExpr("empty", empty, params)}`,
        ];
        const source = `TypesRow { ${fields.join(", ")} }.insert()`;
        return Object.keys(params).length > 0 ? { source, params } : { source };
    });
}
