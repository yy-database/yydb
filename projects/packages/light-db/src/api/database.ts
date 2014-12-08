/**
 * Shared schema and open types.
 *
 * These types are the stable contract used by both the WASM-backed
 * {@link Database} (主线) and the pure-TS {@link PureTsDatabase} (参考实现).
 */

export interface SchemaVersion {
    version: number;
    document: string;
}

/** One row from a scan. */
export interface ScanRow {
    key: string;
    value: Uint8Array;
}

/** Options for scan. */
export interface ScanOptions {
    prefix?: string;
    after?: string | null;
    limit?: number;
}

/** One batch change. */
export interface BatchChange {
    key: string;
    value: Uint8Array | string | null;
}

/** Options for migrateSchema. */
export interface MigrateOptions {
    renames?: ReadonlyArray<
        | { fromType: string; toType: string }
        | { type: string; fromField: string; toField: string }
    >;
}

/** Options for opening any light-db backend. */
export interface OpenOptions {
    /** Use an in-memory session (test only; default is OPFS persistent). */
    memory?: boolean;
}

// ---------------------------------------------------------------------------
// Pure-TS Database (reference / test-only — NOT production path)
// ---------------------------------------------------------------------------

import { StorageEngine } from '../core/engine.js';
import type { StorageEngineOptions } from '../core/engine.js';
import { LightDbError, LightDbErrorCodes } from '../errors/index.js';
import { isOpfsAvailable, OpfsStorageAdapterFactory } from '../storage/opfs/OpfsStorageAdapter.js';
import type { StorageAdapterFactory } from '../storage/StorageAdapter.js';
import { decodeSchema, encodeSchema, parseSchemaDocument, RESERVED_KEY_PREFIX, SCHEMA_KEY } from './schema.js';
import { Transaction } from './transaction.js';
import type { LightDatabase, LightDatabaseOptions } from './types.js';

/** Max database name length. */
const MAX_DATABASE_NAME_LENGTH = 128;

/**
 * Pure-TypeScript database — reference / test backend only.
 *
 * ⚠️ This is NOT the production path. The main {@link Database} uses
 * `@yydb/yydb-unknown-wasm32` for Rust-level semantics, schema validation,
 * VOS execution and OPFS persistence. This class re-implements only a
 * minimal KV schema-store over StorageEngine and is kept for unit tests
 * and as a fallback when WASM initialization is intentionally skipped.
 */
export class PureTsDatabase implements LightDatabase {
    /** Database name. */
    public readonly name: string;

    private readonly engine: StorageEngine;
    private readonly transaction: Transaction;

    private constructor(name: string, engine: StorageEngine) {
        this.name = name;
        this.engine = engine;
        this.transaction = new Transaction(engine);
    }

    /** Open a database with the pure-TS StorageEngine backend. */
    public static async open(name: string, options: LightDatabaseOptions = {}): Promise<PureTsDatabase> {
        if (name.trim().length === 0 || name.length > MAX_DATABASE_NAME_LENGTH) {
            throw new LightDbError(LightDbErrorCodes.unsupported, `database name must be 1-${MAX_DATABASE_NAME_LENGTH} characters`, {
                operation: 'open',
            });
        }

        const storage = resolveStorageFactory(options);
        const engineOptions: StorageEngineOptions = {};
        if (options.pageSize !== undefined) engineOptions.pageSize = options.pageSize;
        if (options.walThreshold !== undefined) engineOptions.walThreshold = options.walThreshold;
        if (options.autoCheckpoint !== undefined) engineOptions.autoCheckpoint = options.autoCheckpoint;
        if (options.lease !== undefined) engineOptions.lease = options.lease;

        const engine = await StorageEngine.open(name, storage, engineOptions);
        return new PureTsDatabase(name, engine);
    }

    /** Read the persisted schema, or `null`. */
    public async getSchema(): Promise<SchemaVersion | null> {
        const value = await this.engine.get(SCHEMA_KEY);
        return value ? decodeSchema(value) : null;
    }

    /**
     * Declare the schema document.
     *
     * The document is the single source of truth: subsequent calls must
     * pass the exact same document. This pure-TS implementation does NOT
     * perform full VOS validation — it only checks that the document is
     * parseable and stores it as KV.
     */
    public async ensureSchema(...input: [string] | string[]): Promise<void> {
        const document = input[0];
        if (typeof document !== 'string') {
            throw new LightDbError(LightDbErrorCodes.schemaInvalid, 'pure-TS backend only accepts raw schema strings', {
                operation: 'ensureSchema',
            });
        }
        const version = parseSchemaDocument(document);
        const existing = await this.getSchema();

        if (existing) {
            if (existing.document !== document) {
                throw new LightDbError(
                    LightDbErrorCodes.schemaConflict,
                    `schema document conflict: stored version ${existing.version}, incoming version ${version}`,
                    { operation: 'ensureSchema' },
                );
            }
            return;
        }

        await this.engine.put(SCHEMA_KEY, encodeSchema({ version, document }));
        this.engine.notifySchemaChange();
    }

    /** Read one record, or `null`. */
    public async get(key: string): Promise<Uint8Array | null> {
        return this.engine.get(this.assertUserKey(key, 'kvGet'));
    }

    /** Insert or replace one record. */
    public async put(key: string, value: Uint8Array | string): Promise<void> {
        const bytes = typeof value === 'string' ? new TextEncoder().encode(value) : value;
        await this.engine.put(this.assertUserKey(key, 'kvPut'), bytes);
    }

    /** Delete one record. Silently succeeds when absent. */
    public async delete(key: string): Promise<void> {
        await this.engine.delete(this.assertUserKey(key, 'kvDelete'));
    }

    /** Begin an explicit transaction. */
    public async begin(): Promise<void> {
        await this.transaction.begin();
    }

    /** Commit the current transaction. */
    public async commit(): Promise<void> {
        await this.transaction.commit();
    }

    /** Roll back the current transaction. */
    public async rollback(): Promise<void> {
        await this.transaction.rollback();
    }

    /** Fold the WAL into the main storage file. */
    public async checkpoint(): Promise<void> {
        await this.engine.checkpoint();
    }

    /** Engine version — pure-TS backend reports no specific engine. */
    public serverVersion(): string {
        return 'light-db-pure-ts';
    }

    /** Basic diagnostics — pure-TS backend only reports the database name. */
    public info(): string {
        return `name=${this.name}\nbackend=pure-ts\n`;
    }

    /** VOS query not available on pure-TS backend. */
    public async query<T = Record<string, unknown>>(_source: string): Promise<T[]> {
        throw new LightDbError(LightDbErrorCodes.unsupported, 'VOS query is not available on the pure-TS backend', {
            operation: 'query',
        });
    }

    /** VOS write programs not available on pure-TS backend. */
    public async execute(_source: string): Promise<void> {
        throw new LightDbError(LightDbErrorCodes.unsupported, 'VOS execute is not available on the pure-TS backend', {
            operation: 'execute',
        });
    }

    /** Close the database and release the StorageEngine. */
    public async close(): Promise<void> {
        if (this.transaction.isActive) {
            await this.transaction.rollback();
        }
        await this.engine.close();
    }

    /** Validate user-supplied keys. */
    private assertUserKey(key: string, operation: string): string {
        if (key.length === 0) {
            throw new LightDbError(LightDbErrorCodes.unsupported, 'key must not be empty', { operation });
        }
        if (key.startsWith(RESERVED_KEY_PREFIX)) {
            throw new LightDbError(LightDbErrorCodes.unsupported, `key prefix "${RESERVED_KEY_PREFIX}" is reserved for the database`, {
                operation,
            });
        }
        return key;
    }
}

/** Resolve the storage backend factory. */
function resolveStorageFactory(options: LightDatabaseOptions): StorageAdapterFactory {
    if (options.storage) {
        return options.storage;
    }
    if (!isOpfsAvailable()) {
        throw new LightDbError(LightDbErrorCodes.unsupported, 'OPFS is unavailable, pass an explicit storage adapter', {
            operation: 'open',
        });
    }
    return new OpfsStorageAdapterFactory();
}
