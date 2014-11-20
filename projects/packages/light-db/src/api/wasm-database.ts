import {
    initWasm,
    openPersistentSession,
    createMemorySession,
    type WasmSession,
} from '@yydb/yydb-unknown-wasm32';
import { emitSource, type EmittableItem } from '@voml/vos';
import { LightDbError } from '../errors/index.js';
import type { BatchChange, MigrateOptions, OpenOptions, ScanOptions, ScanRow, SchemaVersion } from './database.js';

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
 * Browser/WASM 侧 YYDB 数据库句柄（可选后端）。
 *
 * 这是 `@yydb/light-db` 的 WASM 路径。默认可选——包主线是 {@link Database}
 * （`api/database.ts`，纯 TS，无 WASM 依赖），宿主按需显式 import 本文件以引入 WASM。
 */
export class WasmDatabase {
    /** 数据库名（OPFS 路径）。 */
    public readonly name: string;

    private readonly session: WasmSession;

    private constructor(name: string, session: WasmSession) {
        this.name = name;
        this.session = session;
    }

    /** 初始化 WASM 并打开一个 session（memory 或 OPFS persistent）。 */
    public static async open(name: string, options: OpenOptions = {}): Promise<WasmDatabase> {
        if (!name || name.length === 0) {
            throw new Error('database name must not be empty');
        }

        await initWasm();

        const session: WasmSession = options.memory
            ? createMemorySession()
            : openPersistentSession(name);

        return new WasmDatabase(name, session);
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

/** 打开浏览器 WASM 数据库的便捷函数（等价于 {@link WasmDatabase.open}）。 */
export async function openLightDatabase(
    name: string,
    options: OpenOptions = {},
): Promise<WasmDatabase> {
    return WasmDatabase.open(name, options);
}
