import {
    initWasm,
    openPersistentSession,
    createMemorySession,
    type WasmSession,
} from '@yydb/yydb-unknown-wasm32';
import { emitSource, type EmittableItem } from '@voml/vos';
import { LightDbError } from '../errors/index.js';

/** WASM session 通用 JSON 返回形状。 */
interface WasmResult {
    ok: boolean;
    rows?: unknown[];
    value?: unknown;
    schema?: { version: number; document: string } | null;
    success?: boolean;
    error?: unknown;
    code?: unknown;
}

/** Schema 版本与文档。 */
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

/** 打开浏览器数据库的选项。 */
export interface OpenOptions {
    /** 使用内存 session（测试专用；默认使用 OPFS 持久 session）。 */
    memory?: boolean;
}

/** Transaction-local handle for callback-style transactions. */
export class TransactionHandle {
    constructor(private readonly session: WasmSession) {}

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

/**
 * Browser/WASM 侧 YYDB 数据库句柄。
 *
 * 与 `@yydb/yydb` 的 {@link Database} 同名——宿主靠 import 来源区分：
 *
 * ```ts
 * import { Database } from '@yydb/yydb';       // Node 原生
 * import { Database } from '@yydb/light-db';   // 浏览器 WASM
 * ```
 */
export class Database {
    /** 数据库名（OPFS 路径）。 */
    public readonly name: string;

    private readonly session: WasmSession;

    private constructor(name: string, session: WasmSession) {
        this.name = name;
        this.session = session;
    }

    /** 初始化 WASM 并打开一个 session（memory 或 OPFS persistent）。 */
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

    /** 解析 WASM JSON 结果，失败时转 LightDbError。 */
    private parseResult<T>(raw: string): T {
        const parsed = JSON.parse(raw) as WasmResult;
        if (!parsed.ok) {
            const err = LightDbError.fromDiagnosticJson(parsed.error);
            throw err ?? new Error(typeof parsed.error === 'string' ? parsed.error : 'wasm error');
        }
        return parsed as T;
    }

    /** 读取已持久化的 schema，或 `null`。 */
    public async getSchema(): Promise<SchemaVersion | null> {
        const raw = this.parseResult<WasmResult>(this.session.getSchema());
        if (!raw.schema) return null;
        return raw.schema;
    }

    /** 声明 schema 文档。 */
    public async ensureSchema(...input: [string] | EmittableItem[]): Promise<void> {
        const document = typeof input[0] === 'string' ? input[0] : emitSource(input as EmittableItem[]);
        this.parseResult(this.session.ensureSchema(document));
    }

    /** 读取键，或 `null`。 */
    public async get(key: string): Promise<Uint8Array | null> {
        const raw = JSON.parse(this.session.get(key)) as WasmResult;
        if (!raw.ok) throw (LightDbError.fromDiagnosticJson(raw.error) ?? new Error(raw.error as string));
        if (raw.value === null || raw.value === undefined) return null;
        if (Array.isArray(raw.value)) return new Uint8Array(raw.value as number[]);
        return new Uint8Array(raw.value as ArrayLike<number>);
    }

    /** 写入键。 */
    public async put(key: string, value: Uint8Array | string): Promise<void> {
        const bytes = typeof value === 'string' ? new TextEncoder().encode(value) : value;
        this.parseResult(this.session.put(key, bytes));
    }

    /** 删除键。 */
    public async delete(key: string): Promise<void> {
        this.parseResult(this.session.delete(key));
    }

    /** 扫描键值对。 */
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

    /** 原子批量写入/删除。 */
    public async batch(changes: readonly BatchChange[]): Promise<void> {
        const body = changes.map((c) => ({
            key: c.key,
            value: c.value === null ? null : typeof c.value === 'string' ? Array.from(new TextEncoder().encode(c.value)) : Array.from(c.value),
        }));
        this.parseResult(this.session.writeBatch(new TextEncoder().encode(JSON.stringify(body))));
    }

    /** Compare-and-swap。 */
    public async compareExchange(
        key: string, expected: Uint8Array | string | null, replacement: Uint8Array | string | null,
    ): Promise<boolean> {
        const toBytes = (v: Uint8Array | string | null): Uint8Array | null =>
            v === null ? null : typeof v === 'string' ? new TextEncoder().encode(v) : v;
        const raw = JSON.parse(this.session.compareExchange(key, toBytes(expected), toBytes(replacement))) as WasmResult;
        if (!raw.ok) throw (LightDbError.fromDiagnosticJson(raw.error) ?? new Error(raw.error as string));
        return raw.success ?? false;
    }

    /** Callback-style 事务：自动 begin/commit，异常自动 rollback。 */
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

    /** 执行 VOS read pipeline，返回 typed rows。 */
    public async query<T = Record<string, unknown>>(source: string): Promise<T[]> {
        const raw = JSON.parse(this.session.queryVos(source)) as WasmResult;
        if (!raw.ok) throw (LightDbError.fromDiagnosticJson(raw.error) ?? new Error(raw.error as string));
        return (raw.rows ?? []) as T[];
    }

    /** 执行 VOS write program。 */
    public async execute(source: string): Promise<void> {
        this.parseResult(this.session.executeVosWrite(source));
    }

    /** 发布新 schema 版本。 */
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

    /** 执行检查点（折叠 WAL）。 */
    public async checkpoint(): Promise<void> {
        this.parseResult(this.session.checkpoint());
    }

    /** 关闭 session。 */
    public async close(): Promise<void> {
        this.session.close();
    }
}

/** 打开浏览器数据库的便捷函数（等价于 {@link Database.open}）。 */
export async function openLightDatabase(
    name: string,
    options: OpenOptions = {},
): Promise<Database> {
    return Database.open(name, options);
}

// ---------------------------------------------------------------------------
// Pure-TS BrowserDatabase (StorageEngine wrapper — browser-friendly, no WASM)
// ---------------------------------------------------------------------------

import { StorageEngine } from '../core/engine.js';
import type { CheckpointResult, StorageEngineOptions, StorageEngineStats } from '../core/engine.js';
import { LightDbErrorCodes } from '../errors/index.js';
import { isOpfsAvailable, OpfsStorageAdapterFactory } from '../storage/opfs/OpfsStorageAdapter.js';
import type { StorageAdapterFactory } from '../storage/StorageAdapter.js';
import { decodeSchema, encodeSchema, parseSchemaDocument, RESERVED_KEY_PREFIX, SCHEMA_KEY } from './schema.js';
import { Transaction } from './transaction.js';
import type { LightDatabase, LightDatabaseOptions } from './types';

/** 数据库名最大长度 */
const MAX_DATABASE_NAME_LENGTH = 128;

/**
 * 浏览器可用的纯 TS 数据库实现。
 *
 * 与 {@link Database}（WASM 包装）同属 `@yydb/light-db` 作者面，但完全不依赖 WASM。
 * 浏览器测试、OPFS 一致性验证、以及不想引入 WASM 捆绑的场景应使用此实现。
 *
 * 默认后端是 `OpfsStorageAdapterFactory`；通过 `options.storage` 可以显式指定。
 */
export class BrowserDatabase implements LightDatabase {
    /** 数据库名 */
    public readonly name: string;

    private readonly engine: StorageEngine;
    private readonly transaction: Transaction;

    private constructor(name: string, engine: StorageEngine) {
        this.name = name;
        this.engine = engine;
        this.transaction = new Transaction(engine);
    }

    /**
     * 打开数据库
     * @param name 数据库名
     * @param options 打开选项
     */
    public static async open(name: string, options: LightDatabaseOptions = {}): Promise<BrowserDatabase> {
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

        return new BrowserDatabase(name, engine);
    }

    /** 读取已持久化的 schema */
    public async schema(): Promise<SchemaVersion | null> {
        const value = await this.engine.get(SCHEMA_KEY);
        return value ? decodeSchema(value) : null;
    }

    /**
     * 声明 schema 文档
     *
     * 文档是唯一真相：首次写入后，重复调用必须传入完全相同的文档。
     */
    public async ensureSchema(document: string): Promise<void> {
        const version = parseSchemaDocument(document);
        const existing = await this.schema();

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

    /** 读取键 */
    public async get(key: string): Promise<Uint8Array | null> {
        return this.engine.get(this.assertUserKey(key, 'kvGet'));
    }

    /** 写入键 */
    public async put(key: string, value: Uint8Array): Promise<void> {
        await this.engine.put(this.assertUserKey(key, 'kvPut'), value);
    }

    /** 删除键 */
    public async delete(key: string): Promise<boolean> {
        return this.engine.delete(this.assertUserKey(key, 'kvDelete'));
    }

    /** 开始事务 */
    public async begin(): Promise<void> {
        await this.transaction.begin();
    }

    /** 提交事务 */
    public async commit(): Promise<void> {
        await this.transaction.commit();
    }

    /** 回滚事务 */
    public async rollback(): Promise<void> {
        await this.transaction.rollback();
    }

    /** 执行检查点 */
    public async checkpoint(): Promise<CheckpointResult> {
        return this.engine.checkpoint();
    }

    /** 统计信息 */
    public stats(): StorageEngineStats {
        return this.engine.getStats();
    }

    /** 关闭数据库 */
    public async close(): Promise<void> {
        if (this.transaction.isActive) {
            await this.transaction.rollback();
        }
        await this.engine.close();
    }

    /**
     * 校验用户键
     */
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

/**
 * 解析存储后端
 */
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
