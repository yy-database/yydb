export interface ReferenceSchema {
    kind: string;
    name: string;
    tableName: string;
    rootPage: number;
    sql: string | null;
}

export interface ReferenceColumn {
    table: string;
    cid: number;
    name: string;
    declaredType: string;
    notNull: boolean;
    default: string | null;
    primaryKey: number;
}

export type ReferenceValue =
    | { kind: "null" }
    | { kind: "integer"; value: number }
    | { kind: "real"; bits: number }
    | { kind: "text"; value: string }
    | { kind: "blob"; value: Buffer };

export interface ReferenceRow {
    rowid: number;
    values: ReferenceValue[];
}

export interface ReferenceTable {
    name: string;
    declaredRows: number;
    rows: ReferenceRow[];
}

export interface ReferenceManifest {
    pageSize: number;
    encoding: string;
    schema: ReferenceSchema[];
    columns: ReferenceColumn[];
    tables: ReferenceTable[];
}

export interface ReferenceFixture {
    pageSize: number;
    bytes: Buffer;
    manifest: ReferenceManifest;
}

export type ExpectedColumn = readonly [number, string, string, boolean, string | null, number];

export const EXPECTED_COLUMNS: Readonly<Record<string, readonly ExpectedColumn[]>> = {
    simple: [
        [0, "id", "INTEGER", false, null, 1],
        [1, "name", "TEXT", true, "'unknown'", 0],
        [2, "note", "TEXT", false, null, 0],
    ],
    implicit: [
        [0, "a", "TEXT", false, null, 0],
        [1, "b", "INTEGER", false, null, 0],
        [2, "c", "REAL", false, null, 0],
    ],
    empty_table: [
        [0, "id", "INTEGER", false, null, 1],
        [1, "v", "TEXT", false, null, 0],
    ],
    types: [
        [0, "id", "INTEGER", false, null, 1],
        [1, "txt", "TEXT", false, null, 0],
        [2, "lead", "TEXT", false, null, 0],
        [3, "r", "REAL", false, null, 0],
        [4, "b", "BLOB", false, null, 0],
        [5, "n", "INTEGER", false, null, 0],
        [6, "empty", "TEXT", false, null, 0],
    ],
    multi: [
        [0, "code", "TEXT", true, null, 1],
        [1, "region", "TEXT", true, null, 2],
        [2, "amount", "REAL", false, "0", 0],
        [3, "note", "TEXT", false, null, 0],
    ],
    wo: [
        [0, "k", "TEXT", true, null, 1],
        [1, "v", "INTEGER", false, null, 0],
    ],
    affinity: [
        [0, "id", "INTEGER", false, null, 1],
        [1, "v", "REAL", false, null, 0],
    ],
    bulk: [
        [0, "label", "TEXT", false, null, 0],
        [1, "n", "INTEGER", false, null, 0],
    ],
};

export function headerEncoding(bytes: Buffer): string {
    if (bytes.length < 64) {
        return "utf8";
    }
    const code = bytes.readUInt32BE(56);
    switch (code) {
        case 1:
            return "utf8";
        case 2:
            return "utf16le";
        case 3:
            return "utf16be";
        default:
            throw new Error(`unexpected SQLite text encoding ${code}`);
    }
}

export function parseManifest(text: string): ReferenceManifest {
    const manifest: ReferenceManifest = {
        pageSize: 0,
        encoding: "",
        schema: [],
        columns: [],
        tables: [],
    };

    for (const line of text.split(/\r?\n/).filter((entry) => entry.length > 0)) {
        const fields = line.split("\t").map((field) => field.trimEnd());
        switch (fields[0]) {
            case "page":
                manifest.pageSize = Number.parseInt(fields[1], 10);
                break;
            case "encoding":
                manifest.encoding = fields[1];
                break;
            case "schema":
                manifest.schema.push({
                    kind: fields[1],
                    name: hexText(fields[2]),
                    tableName: hexText(fields[3]),
                    rootPage: Number.parseInt(fields[4], 10),
                    sql: optionalText(fields[5], "s"),
                });
                break;
            case "column":
                manifest.columns.push({
                    table: hexText(fields[1]),
                    cid: Number.parseInt(fields[2], 10),
                    name: hexText(fields[3]),
                    declaredType: hexText(fields[4]),
                    notNull: fields[5] !== "0",
                    default: optionalText(fields[6], "d"),
                    primaryKey: Number.parseInt(fields[7], 10),
                });
                break;
            case "rows":
                manifest.tables.push({
                    name: hexText(fields[1]),
                    declaredRows: Number.parseInt(fields[2], 10),
                    rows: [],
                });
                break;
            case "row":
                manifest.tables.at(-1)?.rows.push({
                    rowid: Number.parseInt(fields[1], 10),
                    values: fields.slice(2).map(parseValue),
                });
                break;
            default:
                throw new Error(`unexpected manifest record ${fields[0]}`);
        }
    }

    for (const table of manifest.tables) {
        if (table.rows.length !== table.declaredRows) {
            throw new Error(`row count mismatch for ${table.name}`);
        }
    }

    return manifest;
}

function parseValue(field: string): ReferenceValue {
    switch (field[0]) {
        case "n":
            return { kind: "null" };
        case "i":
            return { kind: "integer", value: Number.parseInt(field.slice(1), 10) };
        case "f":
            return { kind: "real", bits: Number.parseInt(field.slice(1), 16) };
        case "t":
            return { kind: "text", value: hexText(field.slice(1)) };
        case "b": {
            const hex = field.slice(1);
            const value = Buffer.alloc(hex.length / 2);
            for (let index = 0; index < value.length; index += 1) {
                value[index] = Number.parseInt(hex.slice(index * 2, index * 2 + 2), 16);
            }
            return { kind: "blob", value };
        }
        default:
            throw new Error(`unexpected manifest value prefix ${field[0]}`);
    }
}

function optionalText(field: string, prefix: string): string | null {
    if (field === "-") {
        return null;
    }
    if (!field.startsWith(prefix)) {
        throw new Error(`expected manifest field prefix ${prefix}`);
    }
    return hexText(field.slice(prefix.length));
}

function hexText(field: string): string {
    if (field.length % 2 !== 0) {
        throw new Error("odd hex field");
    }
    const bytes = Buffer.alloc(field.length / 2);
    for (let index = 0; index < bytes.length; index += 1) {
        bytes[index] = Number.parseInt(field.slice(index * 2, index * 2 + 2), 16);
    }
    return bytes.toString("utf8");
}
