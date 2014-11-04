import fs from 'node:fs';
import path from 'node:path';
import {
    emitSource,
    type EmittableItem,
} from '@voml/vos';
import {
    defineMicro as buildMicro,
    microFingerprint,
    udfWireKind,
    type DefinedMicro,
    type MicroHandle,
    MicroSessionRegistry,
} from '../shared/micro.js';
import { loadNative } from '../native/binding.js';
import type { UdfScalarDescriptor } from '../shared/udf-types.js';
import { YydbError } from '../shared/errors.js';

/** Published schema metadata. */
export interface SchemaVersion {
    version: number;
    document: string;
}

/** Options for opening a local YYDB file. */
export interface OpenOptions {
    /** Create parent directories when missing (default: true). */
    mkdir?: boolean;
}

/** One row from a {@link Database.scan}. */
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

/** One change in a {@link Database.batch}. Use `null` to delete. */
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

/** One consistency finding from {@link Database.doctor}. */
export interface DoctorIssue {
    severity: 'warn' | 'error';
    code: string;
    message: string;
    key_hint?: string | null;
}

/** Per-namespace usage snapshot from {@link Database.doctor}. */
export interface DoctorNamespaceStats {
    prefix: string;
    record_count: number;
    bytes_used: number;
    object_bytes: number;
    quota_max_bytes?: number | null;
    ttl_expired_pending: number;
}

/** Read-only consistency report. */
export interface DoctorReport {
    file_path?: string | null;
    last_commit_sequence: number;
    last_checkpoint_sequence: number;
    checkpoint_lag_records: number;
    wal_bytes_uncheckpointed: number;
    orphan_object_count: number;
    namespaces: DoctorNamespaceStats[];
    issues: DoctorIssue[];
}

/** Summary of a v0 `.yydx` compaction pass. */
export interface CompactReport {
    checkpointed: boolean;
    orphans_reclaimed: number;
}

/**
 * Transaction-local handle passed to {@link Database.transaction} callbacks.
 * All CRUD methods forward to the underlying native handle while the
 * transaction is in flight. Commit/rollback are owned by {@link Database}.
 */
export class TransactionHandle {
    constructor(private readonly native: ReturnType<ReturnType<typeof loadNative>['Database']['open']>) {}

    async get(key: string): Promise<Uint8Array | null> { return this.native.get(key); }
    async put(key: string, value: Uint8Array | string): Promise<void> {
        const bytes = typeof value === 'string' ? new TextEncoder().encode(value) : value;
        this.native.put(key, bytes);
    }
    async delete(key: string): Promise<void> { this.native.delete(key); }
    async scan(options: ScanOptions = {}): Promise<ScanRow[]> {
        const rowsJson = this.native.scanPrefix(options.prefix ?? '', options.after ?? null, options.limit ?? 1000);
        const rows = JSON.parse(rowsJson) as { key: string; value: number[] }[];
        return rows.map((r) => ({ key: r.key, value: new Uint8Array(r.value) }));
    }
    async batch(changes: readonly BatchChange[]): Promise<void> {
        const body = changes.map((c) => ({
            key: c.key,
            value: c.value === null ? null : typeof c.value === 'string' ? Array.from(new TextEncoder().encode(c.value)) : Array.from(c.value),
        }));
        this.native.writeBatch(JSON.stringify(body));
    }
    async compareExchange(
        key: string, expected: Uint8Array | string | null, replacement: Uint8Array | string | null,
    ): Promise<boolean> {
        const toBytes = (v: Uint8Array | string | null): Uint8Array | null =>
            v === null ? null : typeof v === 'string' ? new TextEncoder().encode(v) : v;
        return this.native.compareExchange(key, toBytes(expected), toBytes(replacement));
    }
    async query<T = Record<string, unknown>>(source: string): Promise<T[]> {
        return JSON.parse(this.native.queryVos(source)) as T[];
    }
    async execute(source: string): Promise<void> { this.native.executeVosWrite(source); }
}

/**
 * Node-facing YYDB local database handle.
 *
 * Directly owns a native `yydb::Connection` via the Node-API binding. No
 * child process, no loopback TCP, no wire protocol in the hot path.
 */
export class Database {
    readonly path: string;
    private readonly native: ReturnType<ReturnType<typeof loadNative>['Database']['open']>;
    private readonly microRegistry: MicroSessionRegistry;

    private constructor(
        dbPath: string,
        native: ReturnType<ReturnType<typeof loadNative>['Database']['open']>,
    ) {
        this.path = dbPath;
        this.native = native;
        this.microRegistry = new MicroSessionRegistry();
    }

    /** Open (create when missing) the database at `dbPath`. */
    static async open(dbPath: string, options: OpenOptions = {}): Promise<Database> {
        const resolved = path.resolve(dbPath);
        if (options.mkdir !== false) {
            fs.mkdirSync(path.dirname(resolved), { recursive: true });
        }

        const native = loadNative();
        const db = native.Database.open(resolved);
        return new Database(resolved, db);
    }

    /** Engine version string (same as `yydb --version`). */
    serverVersion(): string {
        return loadNative().version();
    }

    /** Basic diagnostics: path, schema, record count, journal mode. */
    info(): string {
        const lines = [
            `path=${this.native.path() ?? '<memory>'}`,
            `schema.version=${this.native.schemaVersion() ?? ''}`,
            `records=${this.native.recordCount()}`,
            `journal_mode=${this.native.journalMode()}`,
        ];
        return lines.join('\n') + '\n';
    }

    /** Published schema metadata, or `null` before the first schema. */
    async getSchema(): Promise<{ version: number; document: string } | null> {
        const version = this.native.schemaVersion();
        const document = this.native.schemaDocument();
        if (version === null || document === null) {
            return null;
        }
        return { version, document };
    }

    /**
     * Validate and publish a VOS schema document.
     *
     * Accepts either a raw `.vos` source string, one `@voml/vos` descriptor
     * (`v.table()`, `v.class()`) or a list of descriptors. When descriptors
     * are supplied, they are rendered to source via `emitSource` first.
     */
    async ensureSchema(...input: [string] | [string, ...EmittableItem[]] | EmittableItem[]): Promise<void> {
        let document: string;
        if (typeof input[0] === 'string') {
            document = input[0];
        } else {
            document = emitSource(input as EmittableItem[]);
        }
        this.native.ensureSchema(document);
    }

    /** Read one record, or `null` when the key is absent. */
    async get(key: string): Promise<Uint8Array | null> {
        return this.native.get(key);
    }

    /** Insert or replace one record. */
    async put(key: string, value: Uint8Array | string): Promise<void> {
        const bytes = typeof value === 'string' ? new TextEncoder().encode(value) : value;
        this.native.put(key, bytes);
    }

    /** Delete one record by key. Silently succeeds when the key is absent. */
    async delete(key: string): Promise<void> {
        this.native.delete(key);
    }

    /** List key/value pairs in lexical order. `prefix` defaults to `""` (all keys). */
    async scan(options: ScanOptions = {}): Promise<ScanRow[]> {
        const rowsJson = this.native.scanPrefix(
            options.prefix ?? '',
            options.after ?? null,
            options.limit ?? 1000,
        );
        const rows = JSON.parse(rowsJson) as { key: string; value: number[] }[];
        return rows.map((r) => ({ key: r.key, value: new Uint8Array(r.value) }));
    }

    /** Atomically write or delete many keys in one call. */
    async batch(changes: readonly BatchChange[]): Promise<void> {
        const body = changes.map((c) => ({
            key: c.key,
            value:
                c.value === null
                    ? null
                    : typeof c.value === 'string'
                        ? Array.from(new TextEncoder().encode(c.value))
                        : Array.from(c.value),
        }));
        this.native.writeBatch(JSON.stringify(body));
    }

    /**
     * Compare-and-swap: replace or delete `key` only when its current value
     * exactly equals `expected`. Both `expected` and `replacement` may be
     * `null` (meaning absent / delete). Returns `true` when the swap applied.
     */
    async compareExchange(
        key: string,
        expected: Uint8Array | string | null,
        replacement: Uint8Array | string | null,
    ): Promise<boolean> {
        const toBytes = (v: Uint8Array | string | null): Uint8Array | null =>
            v === null ? null : typeof v === 'string' ? new TextEncoder().encode(v) : v;
        return this.native.compareExchange(key, toBytes(expected), toBytes(replacement));
    }

    /** Fold the WAL into the main file and truncate the sidecar. */
    async checkpoint(): Promise<void> {
        this.native.checkpoint();
    }

    /** Checkpoint then copy a self-contained `.yydb` main file to `dest`. */
    async backup(dest: string): Promise<void> {
        this.native.backup(dest);
    }

    /** Copy main + WAL + SHM without checkpoint; opening `dest` replays the WAL tail. */
    async backupWithWal(dest: string): Promise<void> {
        this.native.backupWithWal(dest);
    }

    /** Run read-only consistency probes against the database. */
    async doctor(): Promise<DoctorReport> {
        return JSON.parse(this.native.doctor()) as DoctorReport;
    }

    /** Fold WAL + reclaim orphan blobs (`.yydx` layouts only). */
    async compactYydx(): Promise<CompactReport> {
        return JSON.parse(this.native.compactYydx()) as CompactReport;
    }

    /**
     * Run `fn` inside an explicit transaction. The transaction commits on
     * normal return and rolls back on throw. Nested calls are not supported.
     *
     * ```ts
     * await db.transaction(async (tx) => {
     *     await tx.put('a', '1');
     *     await tx.put('b', '2');
     * });
     * ```
     */
    async transaction<T>(fn: (tx: TransactionHandle) => Promise<T>): Promise<T> {
        this.native.begin();
        const tx = new TransactionHandle(this.native);
        try {
            const result = await fn(tx);
            this.native.commit();
            return result;
        } catch (err) {
            try {
                this.native.rollback();
            } catch {
                // rollback already failed, don't mask the original error
            }
            throw YydbError.fromDiagnosticJson(err instanceof Error ? err.message : String(err)) ?? err;
        }
    }

    /** Execute a VOS read pipeline and return typed rows. */
    async query<T = Record<string, unknown>>(source: string): Promise<T[]> {
        const rowsJson = this.native.queryVos(source);
        return JSON.parse(rowsJson) as T[];
    }

    /** Execute a VOS write program. */
    async execute(source: string): Promise<void> {
        this.native.executeVosWrite(source);
    }

    /**
     * Publish a new schema version, migrating existing records when their
     * shape no longer matches.
     *
     * Accepts raw `.vos` source or `@voml/vos` descriptors (same as
     * {@link Database.ensureSchema}). Use `renames` to tell the engine which
     * old type/field names map to the new ones — records that would otherwise
     * look structurally unrelated get preserved.
     */
    async migrateSchema(
        documentOrSource: string | EmittableItem,
        ...rest: (EmittableItem | MigrateOptions)[]
    ): Promise<void> {
        let document: string;
        let options: MigrateOptions = {};
        const all = [documentOrSource, ...rest];
        const last = all[all.length - 1];
        if (
            last &&
            typeof last === 'object' &&
            !('table' in (last as Record<string, unknown>)) &&
            !('class' in (last as Record<string, unknown>)) &&
            !('emit' in (last as Record<string, unknown>)) &&
            ('renames' in (last as Record<string, unknown>) || Object.keys(last as object).length === 0)
        ) {
            options = (all.pop() as MigrateOptions) ?? {};
        }
        if (typeof all[0] === 'string') {
            document = all[0] as string;
        } else {
            document = emitSource(all as EmittableItem[]);
        }
        const renamesJson = options.renames?.length
            ? JSON.stringify(options.renames)
            : null;
        this.native.migrateSchema(document, renamesJson);
    }

    /** Build a typed session-local micro definition (without registering). */
    defineMicro<TArgs extends readonly UdfScalarDescriptor[], TReturn extends UdfScalarDescriptor>(
        definition: Parameters<typeof buildMicro<TArgs, TReturn>>[0],
    ): DefinedMicro<TArgs, TReturn> {
        return buildMicro(definition);
    }

    /**
     * Install a session-local host micro into both the local registry and the
     * native engine. Host micros never persist across processes.
     */
    async registerMicro<TArgs extends readonly UdfScalarDescriptor[], TReturn extends UdfScalarDescriptor>(
        definition: DefinedMicro<TArgs, TReturn>,
    ): Promise<MicroHandle> {
        const handle = this.microRegistry.register(definition);
        const argTypes = definition.args.map((d) => udfWireKind(d));
        const returnType = udfWireKind(definition.returns);
        this.native.registerMicro(
            definition.name,
            definition.version,
            argTypes,
            returnType,
            handle.hostId,
            handle.functionId,
            handle.implementationVersion,
            microFingerprint(definition.name, definition.version),
        );
        return handle;
    }

    /** Invoke a registered scalar UDF on the native engine. */
    async callScalar(
        name: string,
        version: number,
        args: readonly (string | number | boolean | null)[],
    ): Promise<string | number | boolean | null> {
        const argsJson = JSON.stringify(args);
        const resultJson = this.native.callScalar(name, version, argsJson);
        return JSON.parse(resultJson);
    }

    /** Release the native handle (best-effort; the engine also drops on GC). */
    async close(): Promise<void> {
        // Native connections drop cleanly when the handle is garbage-collected.
        // No explicit close() is exposed from yydb::Connection.
    }
}
