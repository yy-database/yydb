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

export type SqliteTextEncoding = "utf8" | "utf16le" | "utf16be";

function loadSqliteNative() {
    const native = loadNative() as unknown as Record<string, unknown>;
    if (typeof native.SqliteSnapshot !== "function") {
        throw new Error("Native addon lacks the SQLite snapshot reader. Rebuild the native package.");
    }
    return native as unknown as {
        SqliteSnapshot: new (bytes: Buffer, maxSnapshotBytes: number) => SqliteSnapshotBinding;
        SqliteDatabase: { openInMemory(): SqliteDatabaseBinding; open(path: string): SqliteDatabaseBinding };
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
