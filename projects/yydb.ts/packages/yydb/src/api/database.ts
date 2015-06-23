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
import { loadNative, type NativeStatement as NativeStatementHandle } from '../native/binding.js';
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

/** Declared type for a prepared statement parameter slot. */
export type SlotType = 'any' | 'bool' | 'i64' | 'text' | 'null';

/** Tagged JSON form of raw octets on the host wire: `{"$yydb":"bytes","hex":"..."}`. */
interface TaggedBytes {
    readonly $yydb: 'bytes';
    readonly hex: string;
}

/**
 * Serializes one parameter value to its JSON wire text.
 *
 * The wire is JSON, so a `bigint` is written as a bare integer literal instead
 * of going through `Number`, which would round an `i64` outside ±2^53. Octets
 * use the tagged `bytes` form because JSON has no binary literal.
 */
function encodeParamValue(value: unknown): string {
    if (value === null || value === undefined) {
        return 'null';
    }
    if (typeof value === 'boolean') {
        return value ? 'true' : 'false';
    }
    if (typeof value === 'bigint') {
        return value.toString();
    }
    if (typeof value === 'string') {
        return JSON.stringify(value);
    }
    if (typeof value === 'number') {
        if (!Number.isFinite(value)) {
            throw new YydbError('yydb.query.bind', `parameter \`${value}\` is not a finite number`);
        }
        return JSON.stringify(value);
    }
    if (value instanceof Uint8Array) {
        return JSON.stringify({ $yydb: 'bytes', hex: Buffer.from(value).toString('hex') } satisfies TaggedBytes);
    }
    if (Array.isArray(value)) {
        return `[${value.map(encodeParamValue).join(',')}]`;
    }
    if (typeof value === 'object') {
        return `{${Object.entries(value as Record<string, unknown>)
            .map(([key, item]) => `${JSON.stringify(key)}:${encodeParamValue(item)}`)
            .join(',')}}`;
    }
    throw new YydbError('yydb.query.bind', `parameter of type \`${typeof value}\` cannot cross the wire`);
}

function encodeParams(params: Record<string, unknown> | null | undefined): string | null {
    if (params === undefined || params === null) {
        return null;
    }
    return `{${Object.entries(params)
        .map(([name, value]) => `${JSON.stringify(name)}:${encodeParamValue(value)}`)
        .join(',')}}`;
}

function decodeRowValue(value: unknown): unknown {
    if (value === null || typeof value !== 'object' || Array.isArray(value)) {
        return value;
    }
    const tagged = value as Partial<TaggedBytes>;
    if (tagged.$yydb === 'bytes' && typeof tagged.hex === 'string') {
        return new Uint8Array(Buffer.from(tagged.hex, 'hex'));
    }
    return value;
}

function decodeRows<T>(rowsJson: string): T[] {
    const rows = JSON.parse(rowsJson) as Record<string, unknown>[];
    for (const row of rows) {
        for (const key of Object.keys(row)) {
            row[key] = decodeRowValue(row[key]);
        }
    }
    return rows as T[];
}

/**
 * Tagged JSON form of an exact `date` value (days since the Unix epoch,
 * proleptic Gregorian calendar, UTC-free).
 */
export interface DateScalar {
    readonly $yydb: 'date';
    readonly days: number;
}

/** Tagged JSON form of an exact `time` value (nanoseconds since midnight, no zone). */
export interface TimeScalar {
    readonly $yydb: 'time';
    readonly nanos: number;
}

/**
 * Tagged JSON form of an exact `timestamp` value (microseconds since the Unix
 * epoch). Always UTC, encoded as a decimal string to avoid precision loss.
 */
export interface TimestampScalar {
    readonly $yydb: 'timestamp';
    readonly utcMicros: string;
}

/**
 * Tagged JSON form of an exact fixed-point `decimal` value. `unscaled` is the
 * decimal string of the integer significand scaled by `scale`.
 */
export interface DecimalScalar {
    readonly $yydb: 'decimal';
    readonly precision: number;
    readonly scale: number;
    readonly unscaled: string;
}

/** Every exact scalar crossing the JSON wire boundary uses this tagged form. */
export type ScalarValue = DateScalar | TimeScalar | TimestampScalar | DecimalScalar;

/** Declared execution type of a catalog field. */
export type SchemaFieldType =
    | { readonly kind: 'null' | 'bool' | 'i64' | 'text' | 'bytes' | 'file' | 'date' }
    | { readonly kind: 'time'; readonly unit: 'nanosecond'; readonly timezone: 'none' }
    | { readonly kind: 'timestamp'; readonly unit: 'microsecond'; readonly timezone: 'utc' }
    | { readonly kind: 'decimal'; readonly precision: number; readonly scale: number }
    | { readonly kind: 'vector'; readonly dimension: number }
    | { readonly kind: 'record'; readonly schemaId: number; readonly fieldCount: number };

/** One uniqueness rule published by the catalog. */
export interface UniqueConstraintDescriptor {
    readonly name: string;
    readonly columns: string[];
    /** `true` means NULLs never collide (SQL default); `false` means one shared NULL. */
    readonly nullsDistinct: boolean;
}

/** One field in an introspected table. */
export interface SchemaFieldDescriptor {
    readonly name: string;
    readonly fieldId: number;
    readonly virtualField: number;
    readonly type: SchemaFieldType;
}

/** One introspected table with its declared field types and uniqueness rules. */
export interface SchemaTableDescriptor {
    readonly name: string;
    readonly schemaId: number;
    readonly fields: SchemaFieldDescriptor[];
    readonly uniqueConstraints: UniqueConstraintDescriptor[];
}

/** One named parameter descriptor on a {@link Statement}. */
export interface ParameterDescriptor {
    /** Canonical parameter name (no leading `$`). */
    name: string;
    /** Positional slot index within the bound-value slice (0-based). */
    index: number;
    /** Declared slot type; narrowed from surrounding context when possible. */
    slotTy: SlotType;
    /** Number of times this parameter appears in the source. */
    usageCount: number;
}

/** Whether a prepared statement is a read pipeline or a write program. */
export type QueryKind = 'Read' | 'Write';

/** Metadata about a prepared statement — useful for logging and cache keys. */
export interface StatementInfo {
    statementId: string;
    queryKind: QueryKind;
    parameters: ParameterDescriptor[];
    readSet: string[];
    writeSet: string[];
    sourceFingerprint: string;
    source: string;
}

/**
 * A pre-parsed, pre-lowered VOS query plan.
 *
 * Created by {@link Database.prepare}. The actual lowered op tree lives on
 * the native side in a thread-local registry keyed by `statementId`; this
 * class is a thin JS handle. Bind parameters and re-execute via
 * {@link Statement.execute} without re-parsing or re-lowering the source.
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
        private readonly native: NativeStatementHandle,
        private readonly db: unknown,
    ) {}

    /** The fingerprint-prefixed unique id for this statement. */
    get statementId(): string { return this.native.statementId(); }

    /** Parameter descriptors — empty array when the statement takes no parameters. */
    get parameters(): ParameterDescriptor[] {
        return JSON.parse(this.native.parametersJson()) as ParameterDescriptor[];
    }

    /** Full statement metadata. */
    get info(): StatementInfo {
        return JSON.parse(this.native.info()) as StatementInfo;
    }

    /** Whether this statement has been closed and can no longer execute. */
    get isClosed(): boolean { return this.closed; }

    /**
     * Execute this prepared statement with optional named parameter bindings.
     *
     * Missing parameters, extraneous parameters or mistyped parameters raise
     * `YydbError` with code `yydb.query.bind`.
     *
     * @param params Named parameter values — keys match the parameter names
     * in the VOS source (`$name` → `{ name: value }`). Omit when the statement
     * takes no parameters.
     */
    async execute<T = Record<string, unknown>>(params?: Record<string, unknown>): Promise<T[]> {
        if (this.closed) {
            throw new YydbError('yydb.statement.closed', 'statement has been closed');
        }
        const paramsJson = encodeParams(params);
        const rowsJson = (this.db as any).queryPrepared(this.native, paramsJson);
        return decodeRows<T>(rowsJson);
    }

    /** Release this statement's plan from the native registry. */
    close(): void {
        if (!this.closed) {
            this.native.close();
            this.closed = true;
        }
    }
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
        return decodeRows<T>(this.native.queryVos(source));
    }
    async execute(source: string, params?: Record<string, unknown>): Promise<void> {
        this.native.executeVosWrite(source, encodeParams(params));
    }
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

    /**
     * Prepare a VOS read pipeline into a reusable query plan.
     *
     * Returns a {@link Statement} handle. The actual lowered op tree lives
     * on the native side in a thread-local registry — you can call
     * `statement.execute({ paramName: value })` cheaply multiple times
     * without re-parsing or re-lowering the source.
     *
     * Prepare requires no schema binding; plan building is a pure parse +
     * lower pass. Execute still needs `ensureSchema` first.
     *
     * ```ts
     * const stmt = await db.prepare('User.all().take($limit).collect()');
     * const rows = await stmt.execute({ limit: 100 });
     * const rows2 = await stmt.execute({ limit: 50 });
     * stmt.close();
     * ```
     */
    async prepare(source: string): Promise<Statement> {
        const nativeStmt = this.native.prepare(source);
        return new Statement(nativeStmt, this.native);
    }

    /** Execute a VOS read pipeline and return typed rows. */
    async query<T = Record<string, unknown>>(source: string): Promise<T[]> {
        return decodeRows<T>(this.native.queryVos(source));
    }

    /**
     * Execute a VOS write program with optional named parameters.
     *
     * `Uint8Array` parameters cross as the tagged `bytes` wire form, so octets
     * reach a `bytes` column verbatim.
     */
    async execute(source: string, params?: Record<string, unknown>): Promise<void> {
        this.native.executeVosWrite(source, encodeParams(params));
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

    /**
     * Release the native file handle and its writer lock.
     *
     * The handle is also released when the object is garbage-collected, but
     * callers that must reopen the same file in the same process (or hand it
     * to another process) need this deterministic release point. Methods
     * called after `close()` fail with a closed-handle error.
     */
    async close(): Promise<void> {
        this.native.close();
    }
}
