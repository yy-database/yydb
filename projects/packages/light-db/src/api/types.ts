/**
 * Browser database authoring-surface types.
 *
 * These are the stable contract for `@yydb/light-db`:
 * - Binary values are always `Uint8Array` — never `Buffer`.
 * - No local file paths, file handles, or Node-exclusive types leak through.
 * - Aligned with the Node-side `@yydb/yydb` method names and return semantics.
 */

/** Schema version and document (aligned with `@yydb/yydb`). */
export interface SchemaVersion {
    version: number;
    document: string;
}

/** One row from {@link Database.scan}. */
export interface ScanRow {
    key: string;
    value: Uint8Array;
}

/** Options for {@link Database.scan}. */
export interface ScanOptions {
    /** Include this prefix in every returned key; default `""` returns everything. */
    prefix?: string;
    /** Exclusive continuation cursor from a previous scan's last row; default `null` starts fresh. */
    after?: string | null;
    /** Max rows returned per call; default `1000`. */
    limit?: number;
}

/** One change in {@link Database.batch}. Use `null` to delete. */
export interface BatchChange {
    key: string;
    value: Uint8Array | string | null;
}

/** Options for {@link Database.migrateSchema}. */
export interface MigrateOptions {
    /** Rename entries — `fromType`/`toType` updates a type name; `type`/`fromField`/`toField` updates one field. */
    renames?: ReadonlyArray<
        | { fromType: string; toType: string }
        | { type: string; fromField: string; toField: string }
    >;
}

/** Options for opening the WASM-backed {@link Database}. */
export interface OpenOptions {
    /** Use an in-memory session (test only; default is OPFS persistent). */
    memory?: boolean;
}

/**
 * Light database contract — aligned with the Node-side `@yydb/yydb`.
 * Only the WASM-backed {@link Database} implements this in production;
 * the formerly pure-TS backend has been removed.
 */
export interface LightDatabase {
    /** Database name (OPFS path / namespace). */
    readonly name: string;

    /** Engine version string. */
    serverVersion(): string;

    /** Basic diagnostics — session info from the engine. */
    info(): string;

    /** Published schema metadata, or `null` before the first schema. */
    getSchema(): Promise<SchemaVersion | null>;

    /**
     * Validate and publish a VOS schema document. Accepts raw `.vos`
     * source or `@voml/vos` descriptors (e.g. `v.table()`, `v.class()`).
     */
    ensureSchema(...input: [string] | object[]): Promise<void>;

    /** Read one record, or `null`. */
    get(key: string): Promise<Uint8Array | null>;

    /** Insert or replace one record. */
    put(key: string, value: Uint8Array | string): Promise<void>;

    /** Delete one record. Silently succeeds when absent. */
    delete(key: string): Promise<void>;

    /** Fold the WAL into the main storage. */
    checkpoint(): Promise<void>;

    /** Begin an explicit transaction. */
    begin(): Promise<void>;

    /** Commit the current transaction. */
    commit(): Promise<void>;

    /** Roll back the current transaction. */
    rollback(): Promise<void>;

    /** Execute a VOS read pipeline. */
    query<T = Record<string, unknown>>(source: string): Promise<T[]>;

    /** Execute a VOS write program. */
    execute(source: string): Promise<void>;

    /** Close the session. Idempotent. */
    close(): Promise<void>;
}
