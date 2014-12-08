/**
 * Browser database authoring-surface types.
 *
 * These are the stable contract for `@yydb/light-db`: binary values
 * are always `Uint8Array`, no local file paths, file handles, or
 * Node-exclusive types leak through.
 */

/** Schema version and document (aligned with `@yydb/yydb`). */
export interface SchemaVersion {
    version: number;
    document: string;
}

/**
 * Shared browser-database contract — both the WASM-backed main
 * {@link Database} and the pure-TS {@link PureTsDatabase} implement
 * (or are compatible with) this shape. Node-side parity is maintained
 * via identical method names and return semantics.
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
     * source or `@voml/vos` descriptors.
     */
    ensureSchema(...input: [string] | string[]): Promise<void>;

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

// ---------------------------------------------------------------------------
// Pure-TS backend option types (kept for reference/test use only)
// ---------------------------------------------------------------------------

import type { LeaseMode } from '../core/lease';
import type { StorageAdapterFactory } from '../storage/StorageAdapter';

/**
 * Options for opening the pure-TS {@link PureTsDatabase} backend.
 * Not used by the WASM mainline {@link Database}.
 */
export interface LightDatabaseOptions {
    /** Page size, default 4 KiB. */
    pageSize?: number;
    /** WAL byte threshold after which an auto-checkpoint runs; default 16 MiB. */
    walThreshold?: number;
    /** Whether auto-checkpoint runs; default `true`. */
    autoCheckpoint?: boolean;
    /** Write-lease mode; default `web-locks`. */
    lease?: LeaseMode;
    /** Storage backend; default OPFS. */
    storage?: StorageAdapterFactory;
}
