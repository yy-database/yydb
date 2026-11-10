/**
 * Formal local constructor for the embedded YYDB engine.
 *
 * `Yydb` owns exactly one native engine handle and projects the shared VOS
 * data-author surface (`table` / `prepare` / `transaction` / `getSchema` /
 * `ensureSchema` / `beginDdlSession` / `capabilities`). Every call delegates
 * to the existing N-API primitives through the {@link Database} handle: this
 * module adds options validation, the lifecycle state machine, immutable
 * query builders and the error envelope — never a second parser, checker,
 * resolver or storage.
 */

import type { EmittableItem, FieldDescriptor, Infer, TableDescriptor } from '@voml/vos';
import { Database } from './api/database.js';
import type { Statement, TransactionHandle } from './api/database.js';
import { DatabaseError, type DatabaseErrorExecution, type DatabaseErrorPhase } from './shared/envelope.js';

/** Lifecycle state of a {@link Yydb} instance. Terminal states never revive. */
export type YydbLocalState = 'new' | 'opening' | 'open' | 'closing' | 'closed' | 'failed';

/** Persistent-storage configuration. Omit `storage` for the default backend. */
export interface YydbStorageOptions {
    /** Path of the `.yydb` / `.yydx` file, created when missing. */
    filename: string;
}

/** Options accepted by `new Yydb(options?)`. */
export interface YydbOptions {
    /** File-backed storage. The native binding exposes no in-memory open yet. */
    storage?: YydbStorageOptions;
    /** Open without write access. Not exposed by the native binding yet. */
    readOnly?: boolean;
}

/** Options accepted by {@link Yydb.beginDdlSession}. */
export interface DdlSessionOptions {
    /** Milliseconds before the session refuses further operations. Default 60 000. */
    ttlMs?: number;
}

/** One rename queued by a {@link DdlSession}; mirrors the native rename-map entry. */
type RenameEntry = { fromType: string; toType: string } | { type: string; fromField: string; toField: string };

/** Structured, stable capability report for {@link Yydb.capabilities}. */
export interface YydbCapabilities {
    readonly product: 'yydb';
    readonly deployment: 'local';
    readonly engineVersion: string | null;
    readonly storage: { readonly file: boolean; readonly inMemory: boolean; readonly restore: boolean };
    readonly readOnlyOpen: boolean;
    readonly schema: { readonly ensure: boolean; readonly get: boolean; readonly migrate: 'rename-map' };
    readonly query: {
        readonly dialect: 'vos';
        readonly pipeline: readonly string[];
        readonly terminals: {
            readonly collect: 'native';
            readonly get: 'composed';
            readonly first: 'composed';
            readonly any: 'composed';
            readonly count: 'composed';
        };
        readonly parameters: boolean;
        readonly prepared: boolean;
    };
    readonly transactions: { readonly explicit: boolean; readonly savepoints: boolean; readonly nested: boolean };
    readonly ddl: { readonly session: 'rename-queue'; readonly commitAdvancesRevision: boolean };
    readonly micro: { readonly hostScalar: boolean };
    readonly listen: boolean;
}

/** Application-network listener returned by {@link Yydb.listen} once the native serve primitive is non-blocking. */
export interface YydbListener {
    readonly endpoint: { host: string; port: number };
    close(): Promise<void>;
}

/**
 * Immutable VOS pipeline builder over one table descriptor.
 *
 * `filter` and `map` embed caller-written VOS expression text verbatim — the
 * engine parses and lowers it; the builder never inspects it. `sort_by`,
 * `skip` and `take` compose the canonical pipeline spellings. Terminal
 * methods delegate to `query_vos` / prepared execution on the native handle:
 * `collect` is the native operator, while `get` / `first` / `any` / `count`
 * are composed from `filter` + `take(1)` or a materialised `collect`
 * (the Phase-1 executor has no native `get` / `first` / `any` / `count`).
 */
export class TableQuery<T extends TableDescriptor> {
    private constructor(
        private readonly descriptor: T,
        private readonly steps: readonly string[],
        private readonly db: Database,
    ) {}

    /** @internal Factory used by {@link Yydb.table}. */
    static create<T extends TableDescriptor>(descriptor: T, db: Database): TableQuery<T> {
        return new TableQuery(descriptor, [], db);
    }

    /** Append `.filter(<predicate>)`; `predicate` is raw VOS lambda text, e.g. `x => x.age > 18`. */
    filter(predicate: string): TableQuery<T> {
        return this.append(`.filter(${requireText(predicate, 'filter predicate')})`);
    }

    /** Append `.map(<projection>)`; `projection` is raw VOS text, e.g. `row => row.{ user_name }`. */
    map(projection: string): TableQuery<T> {
        return this.append(`.map(${requireText(projection, 'map projection')})`);
    }

    /** Append `.sort_by(row => row.<field>)` for a descriptor field (or its name). */
    sort_by(key: FieldDescriptor | string): TableQuery<T> {
        return this.append(`.sort_by(${this.keyLambda(key, 'sort_by')})`);
    }

    /** Append `.sort_by_desc(row => row.<field>)` for a descriptor field (or its name). */
    sort_by_desc(key: FieldDescriptor | string): TableQuery<T> {
        return this.append(`.sort_by_desc(${this.keyLambda(key, 'sort_by_desc')})`);
    }

    /** Append `.skip(<count>)`; rows before the window are dropped first. */
    skip(count: number): TableQuery<T> {
        return this.append(`.skip(${requireCount(count, 'skip')})`);
    }

    /** Append `.take(<count>)`; caps the rows the pipeline materialises. */
    take(count: number): TableQuery<T> {
        return this.append(`.take(${requireCount(count, 'take')})`);
    }

    /** Execute the pipeline and materialise every row. `params` binds `$name` slots in embedded text. */
    async collect(params?: Record<string, unknown>): Promise<Infer<T>[]> {
        const source = `${this.descriptor.name}.all()${this.steps.join('')}.collect()`;
        return guard(() => executeRead<Infer<T>>(this.db, source, params), { phase: 'execute', execution: 'replied' });
    }

    /**
     * Fetch the one row whose primary field equals `key`, or `null`.
     * Composed as `.filter(<pk> == $yydb_get_key).take(1).collect()`; extra
     * `params` bind `$name` slots used earlier in the chain.
     */
    async get(key: string | number | bigint, params?: Record<string, unknown>): Promise<Infer<T> | null> {
        const primary = primaryField(this.descriptor);
        const rows = await this
            .filter(`row => row.${primary.name} == $yydb_get_key`)
            .take(1)
            .collect({ yydb_get_key: key, ...params });
        return (rows[0] as Infer<T> | undefined) ?? null;
    }

    /** First row of the window, or `null`. Composed as `.take(1).collect()`. */
    async first(params?: Record<string, unknown>): Promise<Infer<T> | null> {
        const rows = await this.take(1).collect(params);
        return (rows[0] as Infer<T> | undefined) ?? null;
    }

    /** Whether the window has at least one row. Composed as `.take(1).collect()`. */
    async any(params?: Record<string, unknown>): Promise<boolean> {
        const rows = await this.take(1).collect(params);
        return rows.length > 0;
    }

    /**
     * Number of rows in the window. The Phase-1 executor has no native count
     * operator, so this materialises the pipeline and counts — bounded by the
     * engine's budgets, not by a streaming counter.
     */
    async count(params?: Record<string, unknown>): Promise<number> {
        const rows = await this.collect(params);
        return rows.length;
    }

    /**
     * Write one row through the native `Type::insert({ … })` program.
     * Every descriptor field crosses as a named parameter so exact values
     * (bigint, bytes, tagged scalars) reach the engine unmodified.
     */
    async insert(row: Infer<T>): Promise<void> {
        const fields = Object.keys(this.descriptor.fields);
        const body = fields.map((name) => `${name}: $${name}`).join(', ');
        await guard(
            () => this.db.execute(`${this.descriptor.name}::insert({ ${body} })`, row as unknown as Record<string, unknown>),
            { phase: 'execute', execution: 'replied' },
        );
    }

    private append(step: string): TableQuery<T> {
        return new TableQuery(this.descriptor, [...this.steps, step], this.db);
    }

    private keyLambda(key: FieldDescriptor | string, method: string): string {
        const name = typeof key === 'string' ? key : key.name;
        if (typeof name !== 'string' || !name) {
            throw new DatabaseError('yydb.configuration.invalid', `\`${method}\` requires a descriptor field or field name`, {
                phase: 'configuration',
                execution: 'not-sent',
            });
        }
        if (!(name in this.descriptor.fields)) {
            throw new DatabaseError(
                'yydb.configuration.invalid',
                `\`${method}\` names field \`${name}\` which table \`${this.descriptor.name}\` does not declare`,
                { phase: 'configuration', execution: 'not-sent' },
            );
        }
        return `row => row.${name}`;
    }
}

/**
 * Time-bounded DDL session projected onto the native rename-map migration.
 *
 * The engine exposes no incremental DDL plan/apply primitive, so the session
 * queues rename entries (exactly the shapes `migrate_schema` accepts) and
 * publishes them in one `migrateSchema` call at commit. Commit requires the
 * target schema because the native primitive validates the full new document;
 * after a successful commit every prepared statement tracked by the owning
 * {@link Yydb} is invalidated, never silently rebound.
 */
export class DdlSession {
    private readonly renames: RenameEntry[] = [];
    private closed = false;
    private readonly deadline: number;

    /** @internal Factory used by {@link Yydb.beginDdlSession}. */
    constructor(
        private readonly db: Database,
        private readonly invalidatePrepared: () => void,
        ttlMs: number,
    ) {
        this.deadline = Date.now() + ttlMs;
    }

    /** Epoch milliseconds at which the session stops accepting operations. */
    get expiresAt(): number {
        return this.deadline;
    }

    /** Whether the deadline passed without a commit or close. */
    get isExpired(): boolean {
        return !this.closed && Date.now() >= this.deadline;
    }

    /**
     * Queue a field rename: `type.fromField` becomes `toField`. The native
     * rename map preserves record identity; already-stored records keep their
     * original keys on read-back, so the target schema should be published
     * together with this rename at {@link commit}.
     */
    renameField(type: string, fromField: string, toField: string): this {
        this.assertUsable('renameField');
        for (const [label, value] of [['type', type], ['fromField', fromField], ['toField', toField]] as const) {
            if (typeof value !== 'string' || !value) {
                throw new DatabaseError('yydb.configuration.invalid', `renameField \`${label}\` must be a non-empty string`, {
                    phase: 'configuration',
                    execution: 'not-sent',
                });
            }
        }
        this.renames.push({ type, fromField, toField });
        return this;
    }

    /** Queue a type rename: `fromType` becomes `toType`. */
    renameType(fromType: string, toType: string): this {
        this.assertUsable('renameType');
        if (!fromType || !toType) {
            throw new DatabaseError('yydb.configuration.invalid', 'renameType requires non-empty type names', {
                phase: 'configuration',
                execution: 'not-sent',
            });
        }
        this.renames.push({ fromType, toType });
        return this;
    }

    /**
     * Publish `target` together with every queued rename through the native
     * `migrate_schema` primitive, then invalidate tracked prepared statements.
     */
    async commit(target: EmittableItem | readonly EmittableItem[]): Promise<void> {
        this.assertUsable('commit');
        const items = Array.isArray(target) ? target : [target];
        if (items.length === 0) {
            throw new DatabaseError('yydb.configuration.invalid', 'DDL commit requires the target schema', {
                phase: 'configuration',
                execution: 'not-sent',
            });
        }
        try {
            const renames = this.renames.length ? { renames: [...this.renames] } : {};
            await this.db.migrateSchema(items[0], ...items.slice(1), renames);
        } catch (cause) {
            // A publish failure may have advanced the revision before the host
            // saw the reply, so the outcome is not locally decidable.
            throw DatabaseError.from(cause, { phase: 'commit', execution: 'unknown' });
        }
        this.closed = true;
        this.invalidatePrepared();
    }

    /** Discard the session and its queued renames. Idempotent. */
    async close(): Promise<void> {
        if (this.closed) {
            return;
        }
        this.closed = true;
        this.renames.length = 0;
    }

    private assertUsable(operation: string): void {
        if (this.closed) {
            throw new DatabaseError('yydb.ddl.closed', `DDL session already committed or closed; \`${operation}\` refused`, {
                phase: 'execute',
                execution: 'not-sent',
            });
        }
        if (Date.now() >= this.deadline) {
            throw new DatabaseError(
                'yydb.ddl.expired',
                `DDL session expired after its ttl; \`${operation}\` refused — begin a new session`,
                { phase: 'execute', execution: 'not-sent' },
            );
        }
    }
}

/**
 * Embedded YYDB: the formal local constructor over one native engine handle.
 *
 * ```ts
 * const User = v.table('User', { user_id: v.i64().primary(), user_name: v.utf8() });
 * const db = new Yydb({ storage: { filename: './app.yydb' } });
 * await db.open();
 * await db.ensureSchema([User]);
 * await db.table(User).insert({ user_id: 1, user_name: 'ada' });
 * const page = await db.table(User).sort_by(User.user_name).skip(0).take(20).collect();
 * await db.close();
 * ```
 */
export class Yydb {
    /** Lifecycle state; business calls require `'open'`. */
    private state: YydbLocalState = 'new';
    private opening: Promise<void> | null = null;
    private closing: Promise<void> | null = null;
    private inner: Database | null = null;
    private readonly statements = new Set<Statement>();
    private readonly options: YydbOptions;

    /**
     * Save and validate configuration only — nothing touches the engine until
     * {@link open}. A malformed options shape fails at construction time.
     */
    constructor(options: YydbOptions = {}) {
        if (options === null || typeof options !== 'object') {
            throw new DatabaseError('yydb.configuration.invalid', 'Yydb options must be an object', {
                phase: 'configuration',
                execution: 'not-sent',
            });
        }
        if (options.storage !== undefined) {
            const filename = options.storage?.filename;
            if (typeof filename !== 'string' || !filename) {
                throw new DatabaseError('yydb.configuration.invalid', 'options.storage.filename must be a non-empty string', {
                    phase: 'configuration',
                    execution: 'not-sent',
                });
            }
        }
        if (options.readOnly !== undefined && typeof options.readOnly !== 'boolean') {
            throw new DatabaseError('yydb.configuration.invalid', 'options.readOnly must be a boolean', {
                phase: 'configuration',
                execution: 'not-sent',
            });
        }
        this.options = { ...options };
    }

    /** Current lifecycle state. */
    get lifecycle(): YydbLocalState {
        return this.state;
    }

    /**
     * Open the native engine. Concurrent calls merge into one operation;
     * `'closed'` and `'failed'` are terminal and require a fresh instance.
     */
    async open(): Promise<this> {
        if (this.state === 'open') {
            return this;
        }
        if (this.opening) {
            return this.opening.then(() => this);
        }
        if (this.state !== 'new') {
            throw new DatabaseError('yydb.state.invalid', `cannot open a ${this.state} Yydb — construct a new instance`, {
                phase: 'open',
                execution: 'not-sent',
            });
        }
        if (this.options.readOnly === true) {
            this.state = 'failed';
            throw new DatabaseError(
                'yydb.unsupported',
                'read-only open is not exposed by the native binding; the engine would silently keep full write access',
                { phase: 'open', execution: 'not-sent' },
            );
        }
        if (this.options.storage === undefined) {
            this.state = 'failed';
            throw new DatabaseError(
                'yydb.unsupported',
                'the native binding exposes no in-memory open; pass options.storage.filename to persist into a `.yydb` / `.yydx` file',
                { phase: 'open', execution: 'not-sent' },
            );
        }
        this.state = 'opening';
        this.opening = (async () => {
            try {
                this.inner = await Database.open(this.options.storage!.filename);
                this.state = 'open';
            } catch (cause) {
                this.state = 'failed';
                throw DatabaseError.from(cause, { phase: 'open', execution: 'replied' });
            } finally {
                this.opening = null;
            }
        })();
        return this.opening.then(() => this);
    }

    /**
     * Initialise or verify the schema from VOS descriptors — never an implicit
     * migration. A changed document over existing data is refused by the
     * engine; run business migrations through {@link beginDdlSession}.
     */
    async ensureSchema(tables: readonly EmittableItem[]): Promise<void> {
        const db = this.requireOpen('ensureSchema');
        if (!Array.isArray(tables) || tables.length === 0) {
            throw new DatabaseError('yydb.configuration.invalid', 'ensureSchema requires at least one descriptor', {
                phase: 'configuration',
                execution: 'not-sent',
            });
        }
        await guard(() => db.ensureSchema(...(tables as EmittableItem[])), { phase: 'execute', execution: 'replied' });
    }

    /** Published schema metadata, or `null` before the first schema. */
    async getSchema(): Promise<{ version: number; document: string } | null> {
        const db = this.requireOpen('getSchema');
        return guard(() => db.getSchema(), { phase: 'execute', execution: 'replied' });
    }

    /** Query builder over one VOS table descriptor; types derive from `Infer<T>`. */
    table<T extends TableDescriptor>(descriptor: T): TableQuery<T> {
        const db = this.requireOpen('table');
        if (descriptor === null || typeof descriptor !== 'object' || descriptor.kind !== 'table' || typeof descriptor.name !== 'string') {
            throw new DatabaseError('yydb.configuration.invalid', 'table() requires a `v.table()` descriptor', {
                phase: 'configuration',
                execution: 'not-sent',
            });
        }
        return TableQuery.create(descriptor, db);
    }

    /** Prepare a VOS read pipeline into a reusable {@link Statement}; the plan lives on the native side. */
    async prepare(source: string): Promise<Statement> {
        const db = this.requireOpen('prepare');
        const statement = await guard(() => db.prepare(source), { phase: 'execute', execution: 'replied' });
        this.statements.add(statement);
        return statement;
    }

    /**
     * Run `fn` inside one engine-owned transaction. Begin/commit/rollback are
     * native; a commit whose outcome is unclear reports `execution: 'unknown'`.
     */
    async transaction<T>(fn: (tx: TransactionHandle) => Promise<T>): Promise<T> {
        const db = this.requireOpen('transaction');
        try {
            return await db.transaction(fn);
        } catch (cause) {
            if (cause instanceof DatabaseError) {
                throw cause;
            }
            const mapped = toDiagnostic(cause);
            if (!mapped) {
                throw cause; // user callback errors pass through untouched
            }
            const commitStage = mapped.operation === 'commit' || mapped.operation === 'rollback';
            throw DatabaseError.from(mapped, {
                phase: commitStage ? 'commit' : 'execute',
                execution: commitStage ? 'unknown' : 'replied',
            });
        }
    }

    /** Begin a time-bounded DDL session over the native rename-map migration. */
    async beginDdlSession(options: DdlSessionOptions = {}): Promise<DdlSession> {
        const db = this.requireOpen('beginDdlSession');
        const ttlMs = options.ttlMs ?? 60_000;
        if (!Number.isFinite(ttlMs) || ttlMs <= 0) {
            throw new DatabaseError('yydb.configuration.invalid', 'DDL session ttlMs must be a positive number', {
                phase: 'configuration',
                execution: 'not-sent',
            });
        }
        return new DdlSession(db, () => this.invalidateStatements(), ttlMs);
    }

    /**
     * Open the application network for {@link YydbClient}. The native serve
     * primitive is blocking and reopens the file under its own writer lock,
     * so there is no listener handle to attach to an already-open engine.
     * This call fails with `yydb.unsupported` until the binding exposes a
     * non-blocking listener; {@link capabilities} reports `listen: false`.
     */
    async listen(): Promise<YydbListener> {
        this.requireOpen('listen');
        throw new DatabaseError(
            'yydb.unsupported',
            'listen() needs a non-blocking native serve primitive attached to the open connection; the current binding only exposes the blocking `serve()` which takes its own writer lock',
            { phase: 'open', execution: 'not-sent' },
        );
    }

    /** Stable capability structure describing what this deployment can do. */
    capabilities(): YydbCapabilities {
        return {
            product: 'yydb',
            deployment: 'local',
            engineVersion: this.inner ? this.inner.serverVersion() : null,
            storage: { file: true, inMemory: false, restore: true },
            readOnlyOpen: false,
            schema: { ensure: true, get: true, migrate: 'rename-map' },
            query: {
                dialect: 'vos',
                pipeline: ['all', 'filter', 'where', 'map', 'sort_by', 'sort_by_desc', 'skip', 'take', 'collect'],
                terminals: { collect: 'native', get: 'composed', first: 'composed', any: 'composed', count: 'composed' },
                parameters: true,
                prepared: true,
            },
            transactions: { explicit: true, savepoints: true, nested: false },
            ddl: { session: 'rename-queue', commitAdvancesRevision: true },
            micro: { hostScalar: true },
            listen: false,
        };
    }

    /**
     * Cascade-close everything the instance owns: tracked prepared statements,
     * then the native handle and its writer lock. Repeated calls succeed.
     */
    async close(): Promise<void> {
        if (this.state === 'closed') {
            return;
        }
        if (this.closing) {
            return this.closing;
        }
        if (this.state === 'new') {
            this.state = 'closed';
            return;
        }
        this.state = 'closing';
        this.closing = (async () => {
            try {
                if (this.opening) {
                    try {
                        await this.opening;
                    } catch {
                        // opening already failed; the cascade below just settles state
                    }
                }
                this.invalidateStatements();
                if (this.inner) {
                    await this.inner.close();
                    this.inner = null;
                }
                this.state = 'closed';
            } catch (cause) {
                this.state = 'failed';
                throw DatabaseError.from(cause, { phase: 'close', execution: 'unknown' });
            } finally {
                this.closing = null;
            }
        })();
        return this.closing;
    }

    /** `await using` support: closes the engine and everything it owns. */
    [Symbol.asyncDispose](): Promise<void> {
        return this.close();
    }

    private requireOpen(operation: string): Database {
        if (this.state !== 'open' || !this.inner) {
            throw new DatabaseError(
                this.state === 'closed' || this.state === 'failed' ? 'yydb.state.closed' : 'yydb.state.invalid',
                `\`${operation}\` requires an open Yydb (state: ${this.state})`,
                { phase: 'execute', execution: 'not-sent' },
            );
        }
        return this.inner;
    }

    private invalidateStatements(): void {
        for (const statement of this.statements) {
            try {
                statement.close();
            } catch {
                // a plan already released natively is not worth masking the caller's error
            }
        }
        this.statements.clear();
    }
}

async function guard<T>(op: () => Promise<T>, ctx: { phase: DatabaseErrorPhase; execution: DatabaseErrorExecution }): Promise<T> {
    try {
        return await op();
    } catch (cause) {
        if (cause instanceof DatabaseError) {
            throw cause;
        }
        const mapped = toDiagnostic(cause);
        if (!mapped) {
            throw cause;
        }
        throw DatabaseError.from(mapped, ctx);
    }
}

/** Decode a native/YydbError/plain diagnostic into the shared shape, or `null` for user errors. */
function toDiagnostic(cause: unknown): { code: string; message: string; operation?: string; retryable?: boolean; cause?: unknown } | null {
    const direct = DatabaseErrorShape.read(cause);
    if (direct) {
        return direct;
    }
    const parsed = cause instanceof Error ? DatabaseErrorShape.read(cause.message) : DatabaseErrorShape.read(cause);
    return parsed;
}

/** Duck-typed reader for both `YydbError`s and native diagnostic JSON across the two error taxonomies. */
const DatabaseErrorShape = {
    read(value: unknown): { code: string; message: string; operation?: string; retryable?: boolean; cause?: unknown } | null {
        if (value === null || typeof value !== 'object') {
            if (typeof value === 'string') {
                try {
                    return DatabaseErrorShape.read(JSON.parse(value));
                } catch {
                    return null;
                }
            }
            return null;
        }
        const record = value as Record<string, unknown>;
        const code = typeof record.code === 'string' && record.code.startsWith('yydb.') ? record.code : null;
        const message = typeof record.message === 'string' ? record.message : null;
        if (!code || message === null) {
            return null;
        }
        const out: { code: string; message: string; operation?: string; retryable?: boolean; cause?: unknown } = { code, message };
        if (typeof record.operation === 'string') out.operation = record.operation;
        if (typeof record.retryable === 'boolean') out.retryable = record.retryable;
        if (record.cause !== undefined) out.cause = record.cause;
        return out;
    },
};

async function executeRead<T>(db: Database, source: string, params?: Record<string, unknown>): Promise<T[]> {
    if (params && Object.keys(params).length > 0) {
        const statement = await db.prepare(source);
        try {
            return await statement.execute<T>(params);
        } finally {
            statement.close();
        }
    }
    return db.query<T>(source);
}

function requireText(value: string, label: string): string {
    if (typeof value !== 'string' || !value.trim()) {
        throw new DatabaseError('yydb.configuration.invalid', `${label} must be non-empty VOS expression text`, {
            phase: 'configuration',
            execution: 'not-sent',
        });
    }
    return value;
}

function requireCount(value: number, label: string): number {
    if (!Number.isInteger(value) || value < 0) {
        throw new DatabaseError('yydb.configuration.invalid', `\`${label}\` requires a non-negative integer`, {
            phase: 'configuration',
            execution: 'not-sent',
        });
    }
    return value;
}

function primaryField(descriptor: TableDescriptor): FieldDescriptor {
    for (const field of Object.values(descriptor.fields)) {
        if (field.attrs.some((attribute) => attribute.name === 'primary')) {
            return field;
        }
    }
    throw new DatabaseError(
        'yydb.configuration.invalid',
        `table \`${descriptor.name}\` declares no primary field — the engine requires exactly one`,
        { phase: 'configuration', execution: 'not-sent' },
    );
}
