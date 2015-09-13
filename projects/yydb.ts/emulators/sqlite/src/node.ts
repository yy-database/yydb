import { loadNative } from "@yydb/yydb";

/**
 * SQLite read-only binding types — declared locally since yydb-napi does not
 * re-export them through the TS facade. The actual implementations live in
 * the same native `.node` file loaded by `@yydb/yydb`.
 */

export interface SqliteReadLimits {
    maxPages?: number;
    maxRows?: number;
    maxPayloadBytes?: number;
    maxTotalPayloadBytes?: number;
}

export interface SqliteSchemaObject {
    kind: string;
    name: string;
    tableName: string;
    rootPage: number | null;
    sql: string | null;
}

export interface SqliteTableRow {
    rowid: bigint;
    payload: Buffer;
}

export interface SqliteRecordValue {
    kind: string;
    valueInteger?: bigint | null;
    valueReal?: number | null;
    valueText?: string | null;
    valueBlob?: Buffer | null;
}

export interface SqliteDecodedRow {
    rowid: bigint;
    columns: SqliteRecordValue[];
}

export interface SqliteSnapshotBinding {
    schema(limits?: SqliteReadLimits): SqliteSchemaObject[];
    table(name: string, limits?: SqliteReadLimits): SqliteTableRow[];
    tableDecoded(name: string, limits?: SqliteReadLimits): SqliteDecodedRow[];
    sqliteVersion(): string;
    encoding(): string;
    ping(): number;
    pages(): Buffer;
}

export interface SqliteDatabaseBinding {
    openInMemory(): SqliteDatabaseBinding;
    open(path: string): SqliteDatabaseBinding;
    path(): string | null;
    ping(): number;
    sqliteVersion(): string;
    encoding(): string;
    schema(limits?: SqliteReadLimits): SqliteSchemaObject[];
    table(name: string, limits?: SqliteReadLimits): SqliteDecodedRow[];
    tableRaw(name: string, limits?: SqliteReadLimits): SqliteTableRow[];
    pages(): Buffer;
}

export interface SqliteWriterColumnSpec {
    name: string;
    declaredType?: string;
    notNull?: boolean;
    defaultValue?: unknown;
    primaryKey?: boolean;
    unique?: boolean;
}

export interface SqliteWriterBinding {
    createTable(name: string, columnsJson: string): void;
    createIndex(name: string, table: string, columns: string[], unique?: boolean): void;
    insert(table: string, valuesJson: string): number | bigint;
    delete(table: string, rowid: number | bigint): boolean;
    update(table: string, rowid: number | bigint, slotsJson: string): boolean;
    begin(): void;
    commit(): void;
    rollback(): void;
    save(path: string): void;
    pages(): Buffer;
    pageSize(): number;
    schema(limits?: SqliteReadLimits): SqliteSchemaObject[];
    tableDecoded(name: string, limits?: SqliteReadLimits): SqliteDecodedRow[];
}

/** Values accepted by the writer: NULL, integer, real, text or octets. */
export type SqliteWriteValue = null | number | bigint | string | Uint8Array;

/** One update slot: keep the stored value or replace it. */
export type SqliteUpdateSlot = { keep: true } | { set: SqliteWriteValue };

export type SqliteTextEncoding = "utf8" | "utf16le" | "utf16be";

function loadSqliteNative() {
    const native = loadNative() as unknown as Record<string, unknown>;
    if (typeof native.SqliteSnapshot !== "function") {
        throw new Error("Native addon lacks the SQLite snapshot reader. Rebuild the native package.");
    }
    return native as unknown as {
        SqliteSnapshot: new (bytes: Buffer, maxSnapshotBytes: number) => SqliteSnapshotBinding;
        SqliteDatabase: { openInMemory(): SqliteDatabaseBinding; open(path: string): SqliteDatabaseBinding };
        SqliteWriter: { create(pageSize?: number): SqliteWriterBinding; open(path: string): SqliteWriterBinding };
        decodeRecord: (payload: Buffer, encoding?: string) => SqliteRecordValue[];
    };
}

/** Read-only, copied main-file bytes. WAL/journal recovery is not performed. */
export class SqliteSnapshot {
    readonly #native: SqliteSnapshotBinding;

    constructor(bytes: Uint8Array, maxSnapshotBytes = 256 * 1024 * 1024) {
        unsignedLimit(maxSnapshotBytes, "maxSnapshotBytes");
        if (!(bytes instanceof Uint8Array)) {
            throw new TypeError("SQLite snapshot must be a Uint8Array or Buffer");
        }
        if (bytes.byteLength > maxSnapshotBytes) {
            throw new RangeError("sqlite snapshot byte limit exceeded");
        }
        const native = loadSqliteNative();
        this.#native = new native.SqliteSnapshot(Buffer.from(bytes), maxSnapshotBytes);
    }

    schema(limits?: SqliteReadLimits): SqliteSchemaObject[] {
        return this.#native.schema(validateLimits(limits));
    }

    table(name: string, limits?: SqliteReadLimits): SqliteTableRow[] {
        if (typeof name !== "string") {
            throw new TypeError("SQLite table name must be a string");
        }
        return this.#native.table(name, validateLimits(limits));
    }

    /** Reads rows with decoded storage-class values. */
    tableDecoded(name: string, limits?: SqliteReadLimits): SqliteDecodedRow[] {
        if (typeof name !== "string") {
            throw new TypeError("SQLite table name must be a string");
        }
        return this.#native.tableDecoded(name, validateLimits(limits));
    }

    /** SQLite library version stamped in the snapshot header. */
    sqliteVersion(): string {
        return this.#native.sqliteVersion();
    }

    /** Returns the header-claimed text encoding. */
    encoding(): SqliteTextEncoding {
        const name = this.#native.encoding();
        if (name === "utf16le" || name === "utf16be" || name === "utf8") return name;
        throw new Error(`Unexpected SQLite encoding label: ${name}`);
    }

    /** Health probe re-running the full main-file validation. Returns 1 on success. */
    ping(): number {
        return this.#native.ping();
    }

    /** Raw main-file bytes, including the page-size header. */
    pages(): Buffer {
        return this.#native.pages();
    }
}

/**
 * File-backed or blank in-memory SQLite snapshot handle.
 *
 * No WAL or journal recovery is performed and no SQL engine is present.
 * Use this when the path and header meta matter beyond raw main-file bytes.
 */
export class SqliteDatabase {
    readonly #native: SqliteDatabaseBinding;

    private constructor(native: SqliteDatabaseBinding) {
        this.#native = native;
    }

    /** Opens a blank in-memory SQLite format-3 snapshot. */
    static openInMemory(): SqliteDatabase {
        const native = loadSqliteNative();
        return new SqliteDatabase(native.SqliteDatabase.openInMemory());
    }

    /** Opens or creates a file-backed SQLite snapshot (main-file only). */
    static open(path: string): SqliteDatabase {
        if (typeof path !== "string" || path.length === 0) {
            throw new TypeError("SQLite path must be a non-empty string");
        }
        const native = loadSqliteNative();
        return new SqliteDatabase(native.SqliteDatabase.open(path));
    }

    /** File path when file-backed, `null` for in-memory. */
    path(): string | null {
        return this.#native.path();
    }

    /** Health probe re-running the full main-file validation. Returns 1 on success. */
    ping(): number {
        return this.#native.ping();
    }

    /** SQLite library version stamped in the snapshot header. */
    sqliteVersion(): string {
        return this.#native.sqliteVersion();
    }

    /** Returns the header-claimed text encoding. */
    encoding(): SqliteTextEncoding {
        const name = this.#native.encoding();
        if (name === "utf16le" || name === "utf16be" || name === "utf8") return name;
        throw new Error(`Unexpected SQLite encoding label: ${name}`);
    }

    schema(limits?: SqliteReadLimits): SqliteSchemaObject[] {
        return this.#native.schema(validateLimits(limits));
    }

    /** Reads rows with decoded storage-class values from an ordinary rowid table. */
    table(name: string, limits?: SqliteReadLimits): SqliteDecodedRow[] {
        if (typeof name !== "string") {
            throw new TypeError("SQLite table name must be a string");
        }
        return this.#native.table(name, validateLimits(limits));
    }

    /** Reads raw rows (undecoded payload) from an ordinary rowid table. */
    tableRaw(name: string, limits?: SqliteReadLimits): SqliteTableRow[] {
        if (typeof name !== "string") {
            throw new TypeError("SQLite table name must be a string");
        }
        return this.#native.tableRaw(name, validateLimits(limits));
    }

    /** Raw main-file bytes, including the page-size header. */
    pages(): Buffer {
        return this.#native.pages();
    }
}

/**
 * Decodes one SQLite record payload into typed storage-class values.
 *
 * Defaults to `utf8`. Feed the database header encoding (see `encoding()`)
 * when the snapshot advertises utf16le or utf16be.
 */
export function decodeRecord(
    payload: Uint8Array,
    encoding?: SqliteTextEncoding,
): SqliteRecordValue[] {
    if (!(payload instanceof Uint8Array)) {
        throw new TypeError("payload must be a Uint8Array or Buffer");
    }
    const native = loadSqliteNative();
    const encArg: string | undefined =
        encoding === undefined ? undefined : encoding;
    return native.decodeRecord(Buffer.from(payload), encArg);
}

/**
 * Writes SQLite format-3 database files that upstream SQLite implementations
 * open directly: DDL, rows, indexes, deletes, updates and in-memory
 * transactions. The image stays in memory until `save`.
 */
export class SqliteWriter {
    readonly #native: SqliteWriterBinding;

    private constructor(native: SqliteWriterBinding) {
        this.#native = native;
    }

    /** Creates a fresh database image with the given page size (default 4096). */
    static create(pageSize?: number): SqliteWriter {
        const native = loadSqliteNative();
        return new SqliteWriter(native.SqliteWriter.create(pageSize));
    }

    /** Loads an existing SQLite main file for further mutation. */
    static open(path: string): SqliteWriter {
        if (typeof path !== "string" || path.length === 0) {
            throw new TypeError("SQLite path must be a non-empty string");
        }
        const native = loadSqliteNative();
        return new SqliteWriter(native.SqliteWriter.open(path));
    }

    /** Creates a table from structured column specs. */
    createTable(name: string, columns: SqliteWriterColumnSpec[]): void {
        if (typeof name !== "string" || name.length === 0) {
            throw new TypeError("SQLite table name must be a non-empty string");
        }
        if (!Array.isArray(columns) || columns.length === 0) {
            throw new TypeError("SQLite table needs at least one column");
        }
        const payload = columns.map((column) => ({
            name: column.name,
            declaredType: column.declaredType,
            notNull: column.notNull,
            defaultValue: column.defaultValue === undefined ? undefined : encodeWriteValue(column.defaultValue),
            primaryKey: column.primaryKey,
            unique: column.unique,
        }));
        this.#native.createTable(name, JSON.stringify(payload));
    }

    /** Creates an index over one or more columns. */
    createIndex(name: string, table: string, columns: string[], unique = false): void {
        this.#native.createIndex(name, table, columns, unique);
    }

    /** Inserts one row; `undefined` slots apply the column default. Returns the rowid. */
    insert(table: string, values: Array<SqliteWriteValue | undefined>): bigint {
        const payload = values.map((value) => (value === undefined ? { $omit: true } : encodeWriteValue(value)));
        return BigInt(this.#native.insert(table, JSON.stringify(payload)));
    }

    /** Deletes one row by rowid; returns whether a row was removed. */
    delete(table: string, rowid: number | bigint): boolean {
        return this.#native.delete(table, typeof rowid === "bigint" ? rowid : Number(rowid));
    }

    /** Updates one row by rowid; each slot keeps or replaces a column. */
    update(table: string, rowid: number | bigint, slots: SqliteUpdateSlot[]): boolean {
        const payload = slots.map((slot) => ("keep" in slot ? { keep: true } : { set: encodeWriteValue(slot.set) }));
        return this.#native.update(table, typeof rowid === "bigint" ? rowid : Number(rowid), JSON.stringify(payload));
    }

    /** Starts an in-memory transaction. */
    begin(): void {
        this.#native.begin();
    }

    /** Commits the open transaction. */
    commit(): void {
        this.#native.commit();
    }

    /** Rolls the image back to the state before `begin`. */
    rollback(): void {
        this.#native.rollback();
    }

    /** Bumps the change counter and writes the image to `path`. */
    save(path: string): void {
        if (typeof path !== "string" || path.length === 0) {
            throw new TypeError("SQLite path must be a non-empty string");
        }
        this.#native.save(path);
    }

    /** Raw file image bytes. */
    pages(): Buffer {
        return this.#native.pages();
    }

    /** Page size of this database image. */
    pageSize(): number {
        return this.#native.pageSize();
    }

    /** Reads schema objects back through YYDB's own reader. */
    schema(limits?: SqliteReadLimits): SqliteSchemaObject[] {
        return this.#native.schema(validateLimits(limits));
    }

    /** Reads one table's decoded rows back through YYDB's own reader. */
    table(name: string, limits?: SqliteReadLimits): SqliteDecodedRow[] {
        return this.#native.tableDecoded(name, validateLimits(limits));
    }
}

function encodeWriteValue(value: SqliteWriteValue): unknown {
    if (value === null) return null;
    if (typeof value === "bigint") return { $i64: value.toString() };
    if (typeof value === "number") return value;
    if (typeof value === "string") return value;
    if (value instanceof Uint8Array) return { $blob: Buffer.from(value).toString("hex") };
    throw new TypeError("unsupported SQLite write value");
}

function unsignedLimit(value: number, name: string): void {
    if (!Number.isInteger(value) || value < 0 || value > 0xffff_ffff) {
        throw new RangeError(`${name} must be an unsigned 32-bit integer`);
    }
}

function validateLimits(limits?: SqliteReadLimits): SqliteReadLimits | undefined {
    if (limits === undefined) return undefined;
    if (limits === null || typeof limits !== "object" || Array.isArray(limits)) {
        throw new TypeError("SQLite scan limits must be an object");
    }
    const keys = new Set(["maxPages", "maxRows", "maxPayloadBytes", "maxTotalPayloadBytes"]);
    for (const [key, value] of Object.entries(limits)) {
        if (!keys.has(key)) throw new TypeError(`Unknown SQLite scan limit: ${key}`);
        if (value !== undefined) unsignedLimit(value, key);
    }
    return limits;
}
