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

function vosLiteral(value: ReferenceValue): string {
    switch (value.kind) {
        case "null":
            return "null";
        case "integer":
            return String(value.value);
        case "real": {
            const buffer = Buffer.alloc(8);
            buffer.writeBigUInt64BE(BigInt(value.bits));
            return String(buffer.readDoubleBE());
        }
        case "text":
            return JSON.stringify(value.value);
        case "blob":
            return `bytes(${JSON.stringify(Array.from(value.value))})`;
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
                return { kind: "integer", value: Number(column.valueInteger) };
            }
            if (column.kind === "real") {
                const buffer = Buffer.alloc(8);
                buffer.writeDoubleBE(column.valueReal ?? 0);
                return { kind: "real", bits: Number(buffer.readBigUInt64BE()) };
            }
            if (column.kind === "text") {
                return { kind: "text", value: column.valueText ?? "" };
            }
            return { kind: "blob", value: column.valueBlob ?? Buffer.alloc(0) };
        }),
    }));
}

export function buildSimpleAdoptionPrograms(sqlitePath: string): string[] {
    return readTable(sqlitePath, "simple").map((row) => {
        const [, name, note] = row.values;
        return `Simple { id: ${row.rowid}, name: ${vosLiteral(name)}, note: ${vosLiteral(note)} }.insert()`;
    });
}

export function buildTypesAdoptionPrograms(sqlitePath: string): string[] {
    return readTable(sqlitePath, "types").map((row) => {
        const [, txt, lead, r, blob, n, empty] = row.values;
        return `TypesRow { id: ${row.rowid}, txt: ${vosLiteral(txt)}, lead: ${vosLiteral(lead)}, r: ${vosLiteral(r)}, payload: ${vosLiteral(blob)}, n: ${vosLiteral(n)}, empty: ${vosLiteral(empty)} }.insert()`;
    });
}
