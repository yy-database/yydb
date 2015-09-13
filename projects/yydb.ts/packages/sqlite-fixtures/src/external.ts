import { spawnSync } from "node:child_process";

import { referencePython } from "./python.ts";

/** One typed value as an external SQLite implementation returned it. */
export type ExternalValue =
    | { kind: "null" }
    | { kind: "integer"; value: bigint }
    | { kind: "real"; bits: bigint }
    | { kind: "text"; value: string }
    | { kind: "blob"; value: Buffer };

export interface ExternalColumn {
    cid: number;
    name: string;
    declaredType: string;
    notNull: boolean;
    default: string | null;
    primaryKey: number;
}

export interface ExternalIndex {
    name: string;
    unique: boolean;
}

export interface ExternalReport {
    /** `PRAGMA integrity_check` outcome (`ok` or the first error line). */
    integrity: string;
    pageSize: number;
    freelistCount: number;
    schema: Array<{ kind: string; name: string; tableName: string; rootPage: number; sql: string | null }>;
    columns: Record<string, ExternalColumn[]>;
    tables: Record<string, Array<{ rowid: bigint; values: ExternalValue[] }>>;
    indexes: Record<string, ExternalIndex[]>;
}

interface WireRow {
    rowid: number | string;
    values: Array<["n"] | ["i", string] | ["f", string] | ["t", string] | ["b", string]>;
}

interface WireReport {
    integrity: string;
    page_size: number;
    freelist_count: number;
    schema: Array<{ kind: string; name: string; table: string; root: number; sql: string | null }>;
    columns: Record<string, Array<[number, string, string, number, string | null, number]>>;
    tables: Record<string, WireRow[]>;
    indexes: Record<string, Array<{ name: string; unique: number }>>;
}

function decodeWire(wire: WireReport): ExternalReport {
    const report: ExternalReport = {
        integrity: wire.integrity,
        pageSize: wire.page_size,
        freelistCount: wire.freelist_count,
        schema: wire.schema.map((entry) => ({
            kind: entry.kind,
            name: entry.name,
            tableName: entry.table,
            rootPage: entry.root,
            sql: entry.sql,
        })),
        columns: {},
        tables: {},
        indexes: {},
    };
    for (const [table, columns] of Object.entries(wire.columns)) {
        report.columns[table] = columns.map(([cid, name, declaredType, notNull, defaultValue, primaryKey]) => ({
            cid,
            name,
            declaredType,
            notNull: notNull !== 0,
            default: defaultValue,
            primaryKey,
        }));
    }
    for (const [table, rows] of Object.entries(wire.tables)) {
        report.tables[table] = rows.map((row) => ({
            rowid: BigInt(row.rowid),
            values: row.values.map(decodeWireValue),
        }));
    }
    for (const [table, indexes] of Object.entries(wire.indexes)) {
        report.indexes[table] = indexes.map((index) => ({ name: index.name, unique: index.unique !== 0 }));
    }
    return report;
}

function decodeWireValue(value: WireRow["values"][number]): ExternalValue {
    switch (value[0]) {
        case "n":
            return { kind: "null" };
        case "i":
            return { kind: "integer", value: BigInt(value[1]) };
        case "f":
            return { kind: "real", bits: BigInt(`0x${value[1]}`) };
        case "t":
            return { kind: "text", value: value[1] };
        case "b": {
            const bytes = Buffer.alloc(value[1].length / 2);
            for (let index = 0; index < bytes.length; index += 1) {
                bytes[index] = Number.parseInt(value[1].slice(index * 2, index * 2 + 2), 16);
            }
            return { kind: "blob", value: bytes };
        }
    }
}

const PYTHON_READER = `
import json
import sqlite3
import struct
import sys

path = sys.argv[1]
db = sqlite3.connect(f"file:{path}?mode=ro", uri=True)

def enc(value):
    if value is None:
        return ["n"]
    if isinstance(value, int):
        return ["i", str(value)]
    if isinstance(value, float):
        return ["f", struct.pack(">d", value).hex()]
    if isinstance(value, str):
        return ["t", value]
    if isinstance(value, (bytes, bytearray)):
        return ["b", bytes(value).hex()]
    raise TypeError("unexpected SQLite value type: %r" % type(value))

report = {
    "integrity": db.execute("PRAGMA integrity_check").fetchone()[0],
    "page_size": db.execute("PRAGMA page_size").fetchone()[0],
    "freelist_count": db.execute("PRAGMA freelist_count").fetchone()[0],
    "schema": [],
    "columns": {},
    "tables": {},
    "indexes": {},
}
for kind, name, table, root, sql in db.execute(
    "SELECT type, name, tbl_name, rootpage, sql FROM sqlite_master ORDER BY rowid"
):
    report["schema"].append({"kind": kind, "name": name, "table": table, "root": root or 0, "sql": sql})
tables = [
    name
    for (name,) in db.execute(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY rowid"
    )
]
for table in tables:
    quoted = '"' + table.replace('"', '""') + '"'
    report["columns"][table] = [
        [cid, name, declared, not_null, default, pk]
        for cid, name, declared, not_null, default, pk in db.execute('PRAGMA table_info(%s)' % quoted)
    ]
    report["indexes"][table] = [
        {"name": name, "unique": unique}
        for _seq, name, unique, _origin, _partial in db.execute('PRAGMA index_list(%s)' % quoted)
    ]
    rows = []
    for row in db.execute('SELECT rowid, * FROM %s ORDER BY rowid' % quoted):
        rows.append({"rowid": str(row[0]), "values": [enc(value) for value in row[1:]]})
    report["tables"][table] = rows
db.close()
sys.stdout.write(json.dumps(report, ensure_ascii=False))
`;

/** Reads a database file with Python's stdlib `sqlite3`. Returns undefined when Python is unavailable. */
export function readWithPython(path: string): ExternalReport | undefined {
    const python = referencePython();
    if (!python) {
        return undefined;
    }
    const output = spawnSync(python, ["-c", PYTHON_READER, path], {
        timeout: 60_000,
        encoding: "utf8",
        maxBuffer: 256 * 1024 * 1024,
    });
    if (output.error || output.status !== 0) {
        throw new Error(`python sqlite3 reader failed: ${output.stderr || output.stdout}`);
    }
    return decodeWire(JSON.parse(output.stdout) as WireReport);
}

/** Minimal handle of a `better-sqlite3` database for test probes. */
export interface BetterSqlite3Handle {
    prepare(source: string): { run(): unknown; get(...params: unknown[]): unknown };
    close(): void;
}

/** Opens `path` with `better-sqlite3` (test-only dependency of this package). */
export async function openBetterSqlite3(path: string): Promise<BetterSqlite3Handle> {
    const module = await import("better-sqlite3");
    const Constructor = (module.default as { default?: unknown }).default ?? module.default;
    const DatabaseCtor = Constructor as new (path: string, options?: Record<string, unknown>) => BetterSqlite3Handle;
    return new DatabaseCtor(path);
}

/** Reads a database file with `better-sqlite3` (test-only dependency). */
export async function readWithBetterSqlite3(path: string): Promise<ExternalReport> {
    const module = await import("better-sqlite3");
    const Constructor = (module.default as { default?: unknown }).default ?? module.default;
    const DatabaseCtor = Constructor as new (
        path: string,
        options?: Record<string, unknown>,
    ) => BetterSqlite3Handle & {
        pragma(source: string, options?: { simple?: boolean }): unknown;
        defaultSafeIntegers(enabled: boolean): unknown;
    };
    const db = new DatabaseCtor(path, { readonly: true });
    try {
        db.defaultSafeIntegers(true);
        const integrity = db.pragma("integrity_check", { simple: true }) as unknown;
        const pageSize = Number(db.pragma("page_size", { simple: true }));
        const freelistCount = Number(db.pragma("freelist_count", { simple: true }));
        const report: ExternalReport = {
            integrity: typeof integrity === "string" ? integrity : String(integrity),
            pageSize,
            freelistCount,
            schema: [],
            columns: {},
            tables: {},
            indexes: {},
        };
        const master = db
            .prepare("SELECT type, name, tbl_name, rootpage, sql FROM sqlite_master ORDER BY rowid")
            .all() as Array<{ type: string; name: string; tbl_name: string; rootpage: number | bigint | null; sql: string | null }>;
        for (const entry of master) {
            report.schema.push({
                kind: entry.type,
                name: entry.name,
                tableName: entry.tbl_name,
                rootPage: Number(entry.rootpage ?? 0),
                sql: entry.sql,
            });
        }
        const tables = master.filter((entry) => entry.type === "table" && !entry.name.startsWith("sqlite_"));
        for (const table of tables) {
            const quoted = `"${table.name.replace(/"/g, '""')}"`;
            const columns = db.prepare(`PRAGMA table_info(${quoted})`).all() as Array<{
                cid: number;
                name: string;
                type: string;
                notnull: number;
                dflt_value: string | null;
                pk: number;
            }>;
            report.columns[table.name] = columns.map((column) => ({
                cid: Number(column.cid),
                name: column.name,
                declaredType: column.type,
                notNull: Number(column.notnull) !== 0,
                default: column.dflt_value === null || column.dflt_value === undefined ? null : String(column.dflt_value),
                primaryKey: Number(column.pk),
            }));
            const indexes = db.prepare(`PRAGMA index_list(${quoted})`).all() as Array<{ name: string; unique: number }>;
            report.indexes[table.name] = indexes.map((index) => ({ name: index.name, unique: Number(index.unique) !== 0 }));
            const rows = db.prepare(`SELECT rowid AS __rowid, * FROM ${quoted} ORDER BY rowid`).all() as Array<Record<string, unknown>>;
            report.tables[table.name] = rows.map((row) => {
                const values = Object.entries(row)
                    .filter(([key]) => key !== "__rowid")
                    .map(([, value]) => encodeBetterSqliteValue(value));
                return { rowid: BigInt(row.__rowid as number | bigint), values };
            });
        }
        return report;
    } finally {
        db.close();
    }
}

function encodeBetterSqliteValue(value: unknown): ExternalValue {
    if (value === null) return { kind: "null" };
    if (typeof value === "bigint") return { kind: "integer", value };
    if (typeof value === "number") {
        const buffer = Buffer.alloc(8);
        buffer.writeDoubleBE(value);
        return { kind: "real", bits: buffer.readBigUInt64BE() };
    }
    if (typeof value === "string") return { kind: "text", value };
    if (value instanceof Uint8Array) return { kind: "blob", value: Buffer.from(value) };
    throw new Error(`unexpected better-sqlite3 value type ${typeof value}`);
}
