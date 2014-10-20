import {
    initWasm,
    openPersistentSession,
    createMemorySession,
    type WasmSession,
} from '@yydb/yydb-unknown-wasm32';
import { emitSource, type EmittableItem } from '@voml/vos';

/** WASM session 通用 JSON 返回形状。 */
interface WasmResult {
    ok: boolean;
    rows?: unknown[];
    value?: unknown;
    schema?: { version: number; document: string } | null;
    error?: string | null;
    code?: string | null;
}

/** Schema 版本与文档。 */
export interface SchemaVersion {
    version: number;
    document: string;
}

/** 打开浏览器数据库的选项。 */
export interface LightDatabaseOptions {
    /** 使用内存 session（测试专用；默认使用 OPFS 持久 session）。 */
    memory?: boolean;
}

/** 扩展 WASM session 的浏览器端补充方法（Rust 侧已导出）。 */
interface ExtendedSession extends WasmSession {
    delete(key: string): string;
    begin(): string;
    commit(): string;
    rollback(): string;
    checkpoint(): string;
}

/** 浏览器侧 YYDB 数据库句柄，委托给 Rust WASM 核心。 */
export class BrowserDatabase {
    /** 数据库名（OPFS 路径）。 */
    public readonly name: string;

    private readonly session: ExtendedSession;

    private constructor(name: string, session: ExtendedSession) {
        this.name = name;
        this.session = session;
    }

    /** 初始化 WASM 并打开一个 session（memory 或 OPFS persistent）。 */
    public static async open(name: string, options: LightDatabaseOptions = {}): Promise<BrowserDatabase> {
        if (!name || name.length === 0) {
            throw new Error('database name must not be empty');
        }

        await initWasm();

        const base: WasmSession = options.memory
            ? createMemorySession()
            : openPersistentSession(name);

        return new BrowserDatabase(name, base as ExtendedSession);
    }

    /** 读取已持久化的 schema，或 `null`。 */
    public async schema(): Promise<SchemaVersion | null> {
        const raw = JSON.parse(this.session.getSchema()) as WasmResult;
        if (!raw.ok) {
            throw new Error(raw.error ?? 'getSchema failed');
        }
        if (!raw.schema) {
            return null;
        }
        return raw.schema;
    }

    /**
     * 声明 schema 文档。接受原始 `.vos` 源码字符串，或 `@voml/vos`
     * descriptor（`v.table()` / `v.class()`），或多个 descriptor。
     */
    public async ensureSchema(...input: [string] | EmittableItem[]): Promise<void> {
        const document = typeof input[0] === 'string' ? input[0] : emitSource(input as EmittableItem[]);
        const raw = JSON.parse(this.session.ensureSchema(document)) as WasmResult;
        if (!raw.ok) {
            throw new Error(raw.error ?? 'ensureSchema failed');
        }
    }

    /** 读取键，或 `null`。 */
    public async get(key: string): Promise<Uint8Array | null> {
        const raw = JSON.parse(this.session.get(key)) as WasmResult;
        if (!raw.ok) {
            throw new Error(raw.error ?? `get(${key}) failed`);
        }
        if (raw.value === null || raw.value === undefined) {
            return null;
        }
        // WASM JSON 序列化为普通数组 [1, 2, 3, ...]；转回 Uint8Array
        if (raw.value instanceof Uint8Array) {
            return raw.value;
        }
        if (Array.isArray(raw.value)) {
            return new Uint8Array(raw.value as number[]);
        }
        // 兜底：如果是 base64 字符串等，尝试 new Uint8Array
        return new Uint8Array(raw.value as ArrayLike<number>);
    }

    /** 写入键。 */
    public async put(key: string, value: Uint8Array): Promise<void> {
        const raw = JSON.parse(this.session.put(key, value)) as WasmResult;
        if (!raw.ok) {
            throw new Error(raw.error ?? `put(${key}) failed`);
        }
    }

    /** 删除键（底层等价 write_batch([(key, None)])）。 */
    public async delete(key: string): Promise<boolean> {
        const raw = JSON.parse(this.session.delete(key)) as WasmResult;
        if (!raw.ok) {
            throw new Error(raw.error ?? `delete(${key}) failed`);
        }
        return true;
    }

    /** 开启事务。 */
    public async begin(): Promise<void> {
        const raw = JSON.parse(this.session.begin()) as WasmResult;
        if (!raw.ok) {
            throw new Error(raw.error ?? 'begin failed');
        }
    }

    /** 提交事务。 */
    public async commit(): Promise<void> {
        const raw = JSON.parse(this.session.commit()) as WasmResult;
        if (!raw.ok) {
            throw new Error(raw.error ?? 'commit failed');
        }
    }

    /** 回滚事务。 */
    public async rollback(): Promise<void> {
        const raw = JSON.parse(this.session.rollback()) as WasmResult;
        if (!raw.ok) {
            throw new Error(raw.error ?? 'rollback failed');
        }
    }

    /** 执行检查点（折叠 WAL）。 */
    public async checkpoint(): Promise<void> {
        const raw = JSON.parse(this.session.checkpoint()) as WasmResult;
        if (!raw.ok) {
            throw new Error(raw.error ?? 'checkpoint failed');
        }
    }

    /** 数据库统计（暂不可用）。 */
    public stats(): never {
        throw new Error('stats() is not available on the WASM core');
    }

    /** 关闭 session。 */
    public async close(): Promise<void> {
        this.session.close();
    }
}

/** 打开浏览器数据库的便捷函数。 */
export async function openLightDatabase(
    name: string,
    options: LightDatabaseOptions = {},
): Promise<BrowserDatabase> {
    return BrowserDatabase.open(name, options);
}
