import { createRequire } from 'node:module';
import path from 'node:path';

import { Database } from '../api/database.js';

/**
 * Explicit SQLite ETL for the `yydb import` / `yydb export` commands.
 *
 * This is a one-shot transfer over a declared subset, not a storage contract:
 * the SQLite side always carries a single `(key TEXT PRIMARY KEY, value BLOB)`
 * table, which mirrors the YYDB record tree one-to-one. No table-shaped or
 * typed-lossless mapping exists, and no SQLite storage semantics are emulated.
 * The driver is an optional dependency, loaded lazily so the package stays
 * small for projects that never touch `.sqlite` files.
 */

const PAGE_ROWS = 1_000;
const IDENTIFIER = /^[A-Za-z_][A-Za-z0-9_]*$/;
/** Reserved key namespace holding engine metadata; ETL never touches it. */
const RESERVED_PREFIX = '__yydb/';

interface SqliteStatement {
    run(...params: unknown[]): unknown;
    all(...params: unknown[]): unknown[];
}

interface SqliteHandle {
    prepare(sql: string): SqliteStatement;
    exec(sql: string): void;
    transaction<T extends unknown[]>(fn: (...args: T) => void): (...args: T) => void;
    close(): void;
}

type SqliteConstructor = new (file: string, options?: { readonly?: boolean }) => SqliteHandle;

let driver: SqliteConstructor | null = null;

/** Load the optional `better-sqlite3` driver, honouring `YYDB_SQLITE_DRIVER`. */
export function loadSqliteDriver(): SqliteConstructor {
    if (driver) {
        return driver;
    }
    const attempts: (() => SqliteConstructor)[] = [];
    const cwdRequire = () => createRequire(path.join(process.cwd(), 'index.js'));
    const override = process.env.YYDB_SQLITE_DRIVER;
    if (override) {
        attempts.push(() => cwdRequire()(override) as SqliteConstructor);
    }
    attempts.push(() => createRequire(import.meta.url)('better-sqlite3') as SqliteConstructor);
    attempts.push(() => cwdRequire()('better-sqlite3') as SqliteConstructor);

    let lastError: unknown;
    for (const attempt of attempts) {
        try {
            driver = attempt();
            return driver;
        } catch (error) {
            lastError = error;
        }
    }
    throw new Error(
        'yydb import/export needs the optional better-sqlite3 driver: install it in your project '
        + `or point YYDB_SQLITE_DRIVER at an installed copy (${String(lastError)})`,
    );
}

/** Validate an unquoted table name so the ETL commands cannot inject SQL. */
export function tableName(raw?: string): string {
    const name = raw ?? 'records';
    if (!IDENTIFIER.test(name)) {
        throw new Error(`table name must match ${IDENTIFIER}`);
    }
    return name;
}

export interface ExportOptions {
    /** Target table name, defaults to `records`. */
    table?: string;
    /** Key prefix to export; empty exports every record. */
    prefix?: string;
}

export interface ImportOptions {
    /** Source table name, defaults to `records`. */
    table?: string;
    /** Key prefix applied to imported keys. */
    prefix?: string;
}

/**
 * Copy YYDB records into a SQLite table shaped `(key TEXT PRIMARY KEY, value BLOB)`.
 *
 * Existing rows with the same key are replaced, so re-running an export is
 * idempotent. The table is created when missing.
 */
export async function exportToSqlite(
    databasePath: string,
    sqlitePath: string,
    options: ExportOptions = {},
): Promise<{ table: string; rows: number }> {
    const table = tableName(options.table);
    const prefix = options.prefix ?? '';
    if (path.resolve(sqlitePath) === path.resolve(databasePath)) {
        throw new Error('export target must not be the database file');
    }
    if (prefix.startsWith(RESERVED_PREFIX)) {
        throw new Error(`export cannot read the reserved ${RESERVED_PREFIX} namespace`);
    }

    const driver = loadSqliteDriver();
    const database = await Database.open(databasePath);
    let rows = 0;
    const sqlite = new driver(sqlitePath);
    try {
        sqlite.exec(`CREATE TABLE IF NOT EXISTS ${table} (key TEXT PRIMARY KEY, value BLOB NOT NULL)`);
        const insert = sqlite.prepare(`INSERT OR REPLACE INTO ${table} (key, value) VALUES (?, ?)`);
        const write = sqlite.transaction((batch: { key: string; value: Uint8Array }[]) => {
            for (const row of batch) {
                insert.run(row.key, row.value);
            }
        });

        let after: string | undefined;
        while (true) {
            const page = await database.scan({ prefix, after, limit: PAGE_ROWS });
            if (page.length === 0) {
                break;
            }
            const exportable = page.filter((row) => !row.key.startsWith(RESERVED_PREFIX));
            if (exportable.length > 0) {
                write(exportable);
                rows += exportable.length;
            }
            if (page.length < PAGE_ROWS) {
                break;
            }
            after = page[page.length - 1]?.key;
            if (!after) {
                break;
            }
        }
    } finally {
        sqlite.close();
        await database.close();
    }
    return { table, rows };
}

/**
 * Copy a SQLite `(key TEXT PRIMARY KEY, value BLOB)` table into YYDB records.
 *
 * The declared subset is exactly that shape: TEXT non-null keys, BLOB non-null
 * values, no other columns. Anything else is refused instead of coerced, so an
 * import never silently loses SQLite types or constraints.
 */
export async function importFromSqlite(
    sqlitePath: string,
    databasePath: string,
    options: ImportOptions = {},
): Promise<{ table: string; rows: number }> {
    const table = tableName(options.table);
    const prefix = options.prefix ?? '';
    if (path.resolve(sqlitePath) === path.resolve(databasePath)) {
        throw new Error('import source must not be the database file');
    }

    const driver = loadSqliteDriver();
    const sqlite = new driver(sqlitePath, { readonly: true });
    let rows = 0;
    const database = await Database.open(databasePath);
    try {
        const columns = sqlite.prepare(`PRAGMA table_info(${table})`).all() as { name: string; type: string }[];
        if (columns.length !== 2) {
            throw new Error(
                `${table} must have exactly two columns (key TEXT PRIMARY KEY, value BLOB), found ${columns.length}`,
            );
        }
        const names = columns.map((column) => column.name.toLowerCase()).sort();
        if (names[0] !== 'key' || names[1] !== 'value') {
            throw new Error(`${table} columns must be key and value, found ${columns.map((c) => c.name).join(', ')}`);
        }

        const selected = sqlite.prepare(`SELECT key, value FROM ${table} ORDER BY key`).all() as {
            key: unknown;
            value: unknown;
        }[];
        const changes: { key: string; value: Uint8Array }[] = [];
        for (const row of selected) {
            if (typeof row.key !== 'string' || row.key.length === 0) {
                throw new Error(`${table} holds a non-text or empty key, keys must be non-empty TEXT`);
            }
            if (row.key.startsWith(RESERVED_PREFIX) || (prefix && (prefix + row.key).startsWith(RESERVED_PREFIX))) {
                throw new Error(`${table} holds the reserved key ${row.key}, the ${RESERVED_PREFIX} namespace is engine-owned`);
            }
            if (!(row.value instanceof Uint8Array)) {
                throw new Error(
                    `${table} holds a non-BLOB value for key ${row.key}, values must be BLOB (found ${typeof row.value})`,
                );
            }
            changes.push({ key: prefix + row.key, value: row.value });
        }

        for (let start = 0; start < changes.length; start += PAGE_ROWS) {
            const chunk = changes.slice(start, start + PAGE_ROWS);
            await database.batch(chunk);
            rows += chunk.length;
        }
    } finally {
        sqlite.close();
        await database.close();
    }
    return { table, rows };
}
