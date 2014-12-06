/**
 * WASM-backed Database — the production path for `@yydb/light-db`.
 *
 * Delegates to `@yydb/yydb-unknown-wasm32`, which ships a Rust semantic
 * core compiled to WebAssembly. This gives browser users the same VOS
 * schema validation, transaction semantics, OPFS persistence and UDF
 * execution as the Node-side `@yydb/yydb`.
 *
 * ```ts
 * // Node
 * import { Database } from '@yydb/yydb';
 * const db = await Database.open('app.yydb');
 *
 * // Browser
 * import { Database } from '@yydb/light-db';
 * const db = await Database.open('app');
 * ```
 */

import {
    initWasm,
    openPersistentSession,
    createMemorySession,
    yydbVersion,
    type WasmSession,
} from '@yydb/yydb-unknown-wasm32';
import { emitSource, type EmittableItem } from '@voml/vos';
import { LightDbError } from '../errors/index.js';
import type {
    BatchChange,
    MigrateOptions,
    OpenOptions,
    ScanOptions,
    ScanRow,
    SchemaVersion,
} from './database.js';

/** WASM session JSON envelope shape (`{ ok, ..., error }`). */
interface WasmResult {
    ok: boolean;
    rows?: unknown[];
    value?: unknown;
    schema?: { version: number; document: string } | null;
    success?: boolean;
    error?: unknown;
    code?: unknown;
}

// ---------------------------------------------------------------------------
// Transaction-local handle (callback-style)
// ---------------------------------------------------------------------------

/**
 * Transaction-local handle passed to {@link Database.transaction} callbacks.
 * All methods forward to the underlying WASM session; commit/rollback are
 * owned by {@link Database.transaction}.
 */
export class TransactionHandle {
    constructor(private readonly session: WasmSession) {}

    /** Parse a WASM envelope, converting `{ ok: false, error }` to a thrown error. */
    private parseResult<T>(raw: string): T {
        const parsed = JSON.parse(raw) as WasmResult;
        if (!parsed.ok) {
            const err = LightDbError.fromDiagnosticJson(parsed.error);
            throw err ?? new Error(typeof parsed.error === 'string' ? parsed.error : 'wasm error');
        }
        return parsed as T;
    }

    async get(key: string): Promise<Uint8Array | null> {
        const raw = JSON.parse(this.session.get(key)) as WasmResult;
        if (!raw.ok) throw new Error(raw.error as string);
        if (raw.value === null || raw.value === undefined) return null;
        if (Array.isArray(raw.value)) return new Uint8Array(raw.value as number[]);
        return new Uint8Array(raw.value as ArrayLike<number>);
    }

    async put(key: string, value: Uint8Array | string): Promise<void> {
        const bytes = typeof value === 'string' ? new TextEncoder().encode(value) : value;
        this.parseResult(this.session.put(key, bytes));
    }

    async delete(key: string): Promise<void> {
        this.parseResult(this.session.delete(key));
    }

    async scan(options: ScanOptions = {}): Promise<ScanRow[]> {
        const raw = JSON.parse(this.session.scanPrefix(options.prefix ?? '', options.after ?? null, options.limit ?? 1000)) as WasmResult;
        if (!raw.ok) throw new Error(raw.error as string);
        return (raw.rows as { key: string; value: number[] }[] ?? []).map((r) => ({
            key: r.key,
            value: new Uint8Array(r.value),
        }));
    }

    async batch(changes: readonly BatchChange[]): Promise<void> {
        const body = changes.map((c) => ({
            key: c.key,
            value: c.value === null ? null : typeof c.value === 'string' ? Array.from(new TextEncoder().encode(c.value)) : Array.from(c.value),
        }));
        this.parseResult(this.session.writeBatch(new TextEncoder().encode(JSON.stringify(body))));
    }

    async compareExchange(
        key: string, expected: Uint8Array | string | null, replacement: Uint8Array | string | null,
    ): Promise<boolean> {
        const toBytes = (v: Uint8Array | string | null): Uint8Array | null =>
            v === null ? null : typeof v === 'string' ? new TextEncoder().encode(v) : v;
        const raw = JSON.parse(this.session.compareExchange(key, toBytes(expected), toBytes(replacement))) as WasmResult;
        if (!raw.ok) throw new Error(raw.error as string);
        return raw.success ?? false;
    }

    async query<T = Record<string, unknown>>(source: string): Promise<T[]> {
        const raw = JSON.parse(this.session.queryVos(source)) as WasmResult;
        if (!raw.ok) throw new Error(raw.error as string);
        return (raw.rows ?? []) as T[];
    }

    async execute(source: string): Promise<void> {
        this.parseResult(this.session.executeVosWrite(source));
    }
}

// ---------------------------------------------------------------------------
// Database — WASM-backed production path
// ---------------------------------------------------------------------------

/**
 * Browser-local YYDB handle.
 *
 * This is the main {@link Database} export of `@yydb/light-db`. It owns a
 * Rust-backed WASM session (memory or OPFS) and exposes the same user API
 * as the Node-side `@yydb/yydb`.
 */
export class Database {
    /** Database name (OPFS path). */
    public readonly name: string;

    private readonly session: WasmSession;

    private constructor(name: string, session: WasmSession) {
        this.name = name;
        this.session = session;
    }

    /**
     * Initialize WASM and open a session (in-memory or OPFS persistent).
     *
     * @param name Database name. Persistent sessions use it as the OPFS path.
     * @param options Open options — pass `{ memory: true }` for a transient session.
     */
    public static async open(name: string, options: OpenOptions = {}): Promise<Database> {
        if (!name || name.length === 0) {
            throw new Error('database name must not be empty');
        }

        await initWasm();

        const session: WasmSession = options.memory
            ? createMemorySession()
            : openPersistentSession(name);

        return new Database(name, session);
    }

    /** Parse a WASM envelope, converting `{ ok: false, error }` to a thrown error. */
    private parseResult<T>(raw: string): T {
        const parsed = JSON.parse(raw) as WasmResult;
        if (!parsed.ok) {
            const err = LightDbError.fromDiagnosticJson(parsed.error);
            throw err ?? new Error(typeof parsed.error === 'string' ? parsed.error : 'wasm error');
        }
        return parsed as T;
    }

    /** Engine version string (same as `@yydb/yydb` semver). */
    serverVersion(): string {
        return yydbVersion();
    }

    /** Basic diagnostics: session info from the WASM core. */
    info(): string {
        return this.session.info();
    }

    /** Published schema metadata, or `null` before the first schema. */
    public async getSchema(): Promise<SchemaVersion | null> {
        const raw = this.parseResult<WasmResult>(this.session.getSchema());
        if (!raw.schema) return null;
        return raw.schema;
    }

    /**
     * Validate and publish a VOS schema document.
     *
     * Accepts either a raw `.vos` source string, or `@voml/vos` descriptors
     * (`v.table()`, `v.class()`). When descriptors are supplied, they are
     * rendered to source via `emitSource` before being handed to the Rust
     * core for validation.
     */
    public async ensureSchema(...input: [string] | EmittableItem[]): Promise<void> {
        const document = typeof input[0] === 'string' ? input[0] : emitSource(input as EmittableItem[]);
        this.parseResult(this.session.ensureSchema(document));
    }

    /** Read one record, or `null`. */
    public async get(key: string): Promise<Uint8Array | null> {
        const raw = JSON.parse(this.session.get(key)) as WasmResult;
        if (!raw.ok) throw (LightDbError.fromDiagnosticJson(raw.error) ?? new Error(raw.error as string));
        if (raw.value === null || raw.value === undefined) return null;
        if (Array.isArray(raw.value)) return new Uint8Array(raw.value as number[]);
        return new Uint8Array(raw.value as ArrayLike<number>);
    }

    /** Insert or replace one record. */
    public async put(key: string, value: Uint8Array | string): Promise<void> {
        const bytes = typeof value === 'string' ? new TextEncoder().encode(value) : value;
        this.parseResult(this.session.put(key, bytes));
    }

    /** Delete one record. Silently succeeds when the key is absent. */
    public async delete(key: string): Promise<void> {
        this.parseResult(this.session.delete(key));
    }

    /** List key/value pairs in lexical order. */
    public async scan(options: ScanOptions = {}): Promise<ScanRow[]> {
        const raw = JSON.parse(this.session.scanPrefix(
            options.prefix ?? '',
            options.after ?? null,
            options.limit ?? 1000,
        )) as WasmResult;
        if (!raw.ok) throw (LightDbError.fromDiagnosticJson(raw.error) ?? new Error(raw.error as string));
        return (raw.rows as { key: string; value: number[] }[] ?? []).map((r) => ({
            key: r.key,
            value: new Uint8Array(r.value),
        }));
    }

    /** Atomically write or delete many keys in one call. */
    public async batch(changes: readonly BatchChange[]): Promise<void> {
        const body = changes.map((c) => ({
            key: c.key,
            value: c.value === null ? null : typeof c.value === 'string' ? Array.from(new TextEncoder().encode(c.value)) : Array.from(c.value),
        }));
        this.parseResult(this.session.writeBatch(new TextEncoder().encode(JSON.stringify(body))));
    }

    /**
     * Compare-and-swap: replace or delete `key` only when its current value
     * exactly equals `expected`. Both may be `null` (meaning absent / delete).
     */
    public async compareExchange(
        key: string, expected: Uint8Array | string | null, replacement: Uint8Array | string | null,
    ): Promise<boolean> {
        const toBytes = (v: Uint8Array | string | null): Uint8Array | null =>
            v === null ? null : typeof v === 'string' ? new TextEncoder().encode(v) : v;
        const raw = JSON.parse(this.session.compareExchange(key, toBytes(expected), toBytes(replacement))) as WasmResult;
        if (!raw.ok) throw (LightDbError.fromDiagnosticJson(raw.error) ?? new Error(raw.error as string));
        return raw.success ?? false;
    }

    /** Fold the WAL into the main storage. */
    public async checkpoint(): Promise<void> {
        this.parseResult(this.session.checkpoint());
    }

    /** Begin an explicit transaction. Balanced with {@link commit} or {@link rollback}. */
    public async begin(): Promise<void> {
        this.parseResult(this.session.begin());
    }

    /** Commit the current transaction. */
    public async commit(): Promise<void> {
        this.parseResult(this.session.commit());
    }

    /** Roll back the current transaction. */
    public async rollback(): Promise<void> {
        this.parseResult(this.session.rollback());
    }

    /**
     * Run `fn` inside an explicit transaction. Commits on normal return,
     * rolls back on throw. Nested calls are not supported.
     */
    public async transaction<T>(fn: (tx: TransactionHandle) => Promise<T>): Promise<T> {
        this.parseResult(this.session.begin());
        const tx = new TransactionHandle(this.session);
        try {
            const result = await fn(tx);
            this.parseResult(this.session.commit());
            return result;
        } catch (err) {
            try {
                this.session.rollback();
            } catch {
                // don't mask original
            }
            throw err;
        }
    }

    /** Execute a VOS read pipeline and return typed rows. */
    public async query<T = Record<string, unknown>>(source: string): Promise<T[]> {
        const raw = JSON.parse(this.session.queryVos(source)) as WasmResult;
        if (!raw.ok) throw (LightDbError.fromDiagnosticJson(raw.error) ?? new Error(raw.error as string));
        return (raw.rows ?? []) as T[];
    }

    /** Execute a VOS write program. */
    public async execute(source: string): Promise<void> {
        this.parseResult(this.session.executeVosWrite(source));
    }

    /**
     * Publish a new schema version, migrating existing records when their
     * shape no longer matches. Accepts raw `.vos` source or descriptors.
     */
    public async migrateSchema(
        documentOrSource: string | EmittableItem,
        ...rest: (EmittableItem | MigrateOptions)[]
    ): Promise<void> {
        let document: string;
        let options: MigrateOptions = {};
        const all = [documentOrSource, ...rest];
        const last = all[all.length - 1];
        if (
            last && typeof last === 'object' &&
            !('table' in (last as Record<string, unknown>)) &&
            !('class' in (last as Record<string, unknown>)) &&
            ('renames' in (last as Record<string, unknown>) || Object.keys(last as object).length === 0)
        ) {
            options = (all.pop() as MigrateOptions) ?? {};
        }
        if (typeof all[0] === 'string') {
            document = all[0] as string;
        } else {
            document = emitSource(all as EmittableItem[]);
        }
        const renamesJson = options.renames?.length ? JSON.stringify(options.renames) : null;
        this.parseResult(this.session.migrateSchema(document, renamesJson));
    }

    /**
     * Install a session-local host micro UDF.
     *
     * The WASM core exposes `setMicroHostInvoker` to call back into JS for
     * host-provided scalar UDFs. `registerMicro` registers the definition on
     * the Rust side; the invoker function handles the actual JS-side call.
     *
     * @param invoker A JS function that receives `{ hostId, handleVersion, functionId, args }`
     * and returns the scalar result.
     */
    public setMicroHostInvoker(
        invoker: (payload: { hostId: number; handleVersion: number; functionId: string; args: readonly unknown[] }) => unknown,
    ): void {
        this.session.setMicroHostInvoker(invoker as unknown as (...args: unknown[]) => unknown);
    }

    /**
     * Close the session. Idempotent — further calls fail.
     *
     * On persistent (OPFS) sessions this triggers a final snapshot publish.
     */
    public async close(): Promise<void> {
        this.session.close();
    }
}

// ---------------------------------------------------------------------------
// Legacy aliases
// ---------------------------------------------------------------------------

/**
 * @deprecated Use {@link Database} instead. Kept as a compat alias for code
 * that explicitly imported `WasmDatabase`.
 */
export const WasmDatabase = Database;

/** @deprecated Use {@link PureTsDatabase} from `./database` instead. */
export type { PureTsDatabase } from './database';

/** Open a browser WASM database — convenience alias for {@link Database.open}. */
export async function openLightDatabase(name: string, options: OpenOptions = {}): Promise<Database> {
    return Database.open(name, options);
}
