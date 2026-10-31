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
import {
    MicroSessionRegistry,
    defineMicro as buildMicro,
    microFingerprint,
    udfWireKind,
    type DefinedMicro,
    type MicroHandle,
} from '../shared/micro.js';
import type {
    BatchChange,
    DoctorReport,
    LightDatabase,
    MigrateOptions,
    OpenOptions,
    ParameterDescriptor,
    ScanOptions,
    ScanRow,
    SchemaVersion,
    StatementInfo,
} from './types.js';
import type {
    InferUdfArgs,
    InferUdfReturn,
    UdfScalarDescriptor,
} from '../shared/udf-types.js';

/** WASM session JSON envelope shape (`{ ok, ..., error }`). */
interface WasmResult {
    ok: boolean;
    rows?: unknown[];
    value?: unknown;
    schema?: { version: number; document: string } | null;
    success?: boolean;
    report?: unknown;
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
export class Database implements LightDatabase {
    /** Database name (OPFS path). */
    public readonly name: string;

    private readonly session: WasmSession;

    private readonly registry: MicroSessionRegistry;

    private constructor(name: string, session: WasmSession) {
        this.name = name;
        this.session = session;
        this.registry = new MicroSessionRegistry();
        // Route WASM host-micro callbacks into our per-session registry.
        session.setMicroHostInvoker((payload) => this.registry.invokeFromWire(payload));
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
     * Override the host micro invoker.
     *
     * The constructor wires this to the per-session {@link MicroSessionRegistry}
     * so that high-level {@link registerMicro} + {@link callScalar} works out of
     * the box. Call this only if you need to replace the default resolution
     * logic (e.g. to inject a mock invoker in tests).
     */
    public overrideMicroHostInvoker(
        invoker: (payload: {
            hostId: number;
            handleVersion: number;
            functionId: string;
            args: readonly unknown[];
        }) => unknown,
    ): void {
        this.session.setMicroHostInvoker(invoker as unknown as (...args: unknown[]) => unknown);
    }

    /**
     * Build a typed session-local micro definition without registering it.
     *
     * Same signature as the Node-side `@yydb/yydb`; this is a thin alias
     * over the shared `defineMicro` helper so author code stays portable.
     */
    public defineMicro<
        TArgs extends readonly UdfScalarDescriptor[],
        TReturn extends UdfScalarDescriptor,
    >(input: {
        readonly name: string;
        readonly version: number;
        readonly args: TArgs;
        readonly returns: TReturn;
        readonly effect: 'pure';
        readonly deterministic: boolean;
        readonly mode?: 'scalar';
        readonly fn: (...args: InferUdfArgs<TArgs>) => InferUdfReturn<TReturn>;
    }): DefinedMicro<TArgs, TReturn> {
        return buildMicro(input);
    }

    /**
     * Install a session-local host micro into both the JS registry and the
     * WASM core. Host micros never persist across sessions.
     *
     * Uses the high-level JSON bridge (`registerMicroJson`) so we never
     * encode wire bodies in TS; Rust owns the protocol.
     */
    public async registerMicro<
        TArgs extends readonly UdfScalarDescriptor[],
        TReturn extends UdfScalarDescriptor,
    >(definition: DefinedMicro<TArgs, TReturn>): Promise<MicroHandle> {
        const handle = this.registry.register(definition);
        const argTypes = definition.args.map((d) => udfWireKind(d));
        const returnType = udfWireKind(definition.returns);
        const fingerprint = Array.from(microFingerprint(definition.name, definition.version));
        const json = JSON.stringify({
            name: definition.name,
            version: definition.version,
            argTypes,
            returnType,
            hostId: handle.hostId,
            functionId: handle.functionId,
            handleVersion: handle.implementationVersion,
            fingerprint,
        });
        this.parseResult(this.session.registerMicroJson(json));
        return handle;
    }

    /**
     * Invoke a registered scalar UDF via the WASM core.
     *
     * Uses the high-level JSON bridge (`callScalarJson`) — args are encoded
     * as plain JSON scalars (`number`, `string`, `boolean`, `null`).
     */
    public async callScalar(
        name: string,
        version: number,
        args: readonly (string | number | boolean | null)[],
    ): Promise<string | number | boolean | null> {
        const json = JSON.stringify({ name, version, args });
        const raw = JSON.parse(this.session.callScalarJson(json)) as WasmResult;
        if (!raw.ok) throw (LightDbError.fromDiagnosticJson(raw.error) ?? new Error(raw.error as string));
        const value = raw.value;
        if (value === null || value === undefined) return null;
        return value as string | number | boolean;
    }

    /**
     * Close the session. Idempotent — further calls fail.
     *
     * On persistent (OPFS) sessions this triggers a final snapshot publish.
     */
    public async close(): Promise<void> {
        this.session.close();
    }

    // -------------------------------------------------------------------
    // Doctor + Prepared statements
    // -------------------------------------------------------------------

    /**
     * Read-only consistency probe.
     *
     * Returns a report with checkpoint lag, orphan blob counts, per-namespace
     * stats and any named diagnostic issues — same shape as the Node-side
     * `@yydb/yydb` doctor.
     */
    public async doctor(): Promise<DoctorReport> {
        const raw = JSON.parse(this.session.doctorJson()) as WasmResult;
        if (!raw.ok) throw (LightDbError.fromDiagnosticJson(raw.error) ?? new Error(raw.error as string));
        return raw.report as DoctorReport;
    }

    /**
     * Prepare a VOS query plan — parses and lowers once, re-executes many times.
     *
     * ```ts
     * const stmt = await db.prepare('User.all().filter(u => u.age > $min).take($limit)');
     * const rows = await stmt.execute({ min: 18, limit: 50 });
     * stmt.close();
     * ```
     */
    public async prepare(source: string): Promise<Statement> {
        const raw = JSON.parse(this.session.prepareJson(source)) as WasmResult & Partial<StatementInfo>;
        if (!raw.ok) throw (LightDbError.fromDiagnosticJson(raw.error) ?? new Error(raw.error as string));
        const info: StatementInfo = {
            statementId: raw.statementId!,
            queryKind: raw.queryKind!,
            parameters: raw.parameters!,
            readSet: raw.readSet!,
            writeSet: raw.writeSet!,
            sourceFingerprint: raw.sourceFingerprint!,
            source: raw.source!,
        };
        return new Statement(this.session, info);
    }
}

// ---------------------------------------------------------------------------
// Statement — WASM-backed prepared query plan
// ---------------------------------------------------------------------------

/**
 * A pre-parsed, pre-lowered VOS query plan (WASM variant).
 *
 * The lowered op tree lives in the session's internal registry keyed by
 * `statementId`; this class is a thin JS handle. Bind parameters and
 * re-execute via {@link Statement.execute} without re-parsing or re-lowering.
 *
 * ```ts
 * const stmt = await db.prepare('User.all().take($limit).collect()');
 * const rows = await stmt.execute({ limit: 100 });
 * // re-execute with different bind
 * const rows2 = await stmt.execute({ limit: 50 });
 * // release when done
 * stmt.close();
 * ```
 */
export class Statement {
    private closed = false;

    constructor(
        private readonly session: WasmSession,
        private readonly infoObj: StatementInfo,
    ) {}

    /** The unique id for this prepared statement (WASM-assigned). */
    get statementId(): string {
        return this.infoObj.statementId;
    }

    /** Parameter descriptors — empty array when the statement takes no parameters. */
    get parameters(): ParameterDescriptor[] {
        return this.infoObj.parameters;
    }

    /** Full statement metadata — query kind, read/write sets, fingerprint and source. */
    get info(): StatementInfo {
        return this.infoObj;
    }

    /** Whether this statement has been closed and can no longer execute. */
    get isClosed(): boolean {
        return this.closed;
    }

    /**
     * Execute this prepared statement with optional named parameter bindings.
     *
     * Missing parameters, extraneous parameters or mistyped parameters raise
     * an error with `yydb.query.bind` code.
     *
     * @param params Named parameter values — keys match the parameter names
     * in the VOS source (`$name` → `{ name: value }`). Omit when the statement
     * takes no parameters.
     */
    async execute<T = Record<string, unknown>>(params?: Record<string, unknown> | null): Promise<T[]> {
        if (this.closed) {
            throw new Error('yydb.statement.closed: statement has been closed');
        }
        const json = JSON.stringify({
            statementId: this.infoObj.statementId,
            params: params ?? null,
        });
        const raw = JSON.parse(this.session.queryPreparedJson(json)) as WasmResult;
        if (!raw.ok) {
            throw (LightDbError.fromDiagnosticJson(raw.error) ?? new Error(raw.error as string));
        }
        return (raw.rows ?? []) as T[];
    }

    /** Release this statement's plan from the WASM session registry. Idempotent. */
    close(): void {
        if (!this.closed) {
            this.session.closeStatement(this.infoObj.statementId);
            this.closed = true;
        }
    }
}

// ---------------------------------------------------------------------------
// Convenience entry
// ---------------------------------------------------------------------------

/** Open a browser database — convenience alias for {@link Database.open}. */
export async function openLightDatabase(name: string, options: OpenOptions = {}): Promise<Database> {
    return Database.open(name, options);
}
