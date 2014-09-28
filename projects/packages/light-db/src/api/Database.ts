/**
 * 与 `@yydb/yydb` 同构的数据库句柄
 *
 * YYDB 的 Node 产品经原生绑定与线协议提供 `get` / `put` / `ensureSchema` 等作者面；
 * 浏览器产品（Light DB）在这里提供同名同语义的方法，宿主只需切换包名。
 */

import { LightDbError, LightDbErrorCodes } from "../errors";
import { StorageEngine, createStorageEngine } from "../storage";
import type { CheckpointResult, SchemaVersion, StorageEngineOptions, StorageStats } from "../types";

/** Schema 记录在存储引擎中的键名，与用户键空间隔离 */
const SCHEMA_RECORD_KEY = "__yydb__/schema";

/** 内部记录键前缀，不计入 `recordCount` */
const RESERVED_KEY_PREFIXES = ["__yydb__/", "__lightdb__/"] as const;

/** Schema 版本 pragma，与 Rust 侧 `catalog::document_version` 一致 */
const VERSION_PRAGMA = "// @yydb-schema-version:";

/**
 * 打开数据库的配置选项
 */
export interface DatabaseOptions extends StorageEngineOptions {
    /** 数据库名称，默认 `default` */
    dbName?: string;
}

/**
 * 从 Schema 文档的前导注释读取版本号
 *
 * 格式：`// @yydb-schema-version: <u32>`，位于第一个非注释行之前。
 * 未声明时返回 `1`。
 *
 * @param document Schema 文档
 */
export function documentVersion(document: string): number {
    for (const rawLine of document.split("\n")) {
        const line = rawLine.trim();
        if (line.startsWith(VERSION_PRAGMA)) {
            const parsed = Number.parseInt(line.slice(VERSION_PRAGMA.length).trim(), 10);
            return Number.isNaN(parsed) ? 1 : parsed;
        }
        if (line.length > 0 && !line.startsWith("//")) {
            break;
        }
    }
    return 1;
}

/**
 * 持久化在存储引擎中的 Schema 记录
 */
interface SchemaRecord {
    /** Schema 版本 */
    version: number;
    /** Schema 文档全文 */
    document: string;
}

/**
 * 浏览器侧数据库句柄
 *
 * 与 `@yydb/yydb` 的 `Database` 保持同名同语义：`open`、`get`、`put`、`getSchema`、
 * `ensureSchema`、`checkpoint`、`close`。Schema 文档是数据库真相，后续调用必须与原文档完全一致。
 */
export class Database {
    private readonly storageEngine: StorageEngine;
    private readonly dbNameValue: string;
    private closed: boolean;

    /**
     * 创建数据库实例
     * @param storageEngine 已初始化的存储引擎
     * @param dbName 数据库名称
     */
    private constructor(storageEngine: StorageEngine, dbName: string) {
        this.storageEngine = storageEngine;
        this.dbNameValue = dbName;
        this.closed = false;
    }

    /**
     * 打开（创建）数据库
     *
     * @param path 数据库文件所在目录；浏览器适配器接入后为存储命名空间
     * @param options 配置选项
     */
    public static async open(path: string, options: DatabaseOptions = {}): Promise<Database> {
        const dbName = options.dbName ?? "default";
        const engineOptions: StorageEngineOptions = {};

        if (options.pageSize !== undefined) {
            engineOptions.pageSize = options.pageSize;
        }
        if (options.walThreshold !== undefined) {
            engineOptions.walThreshold = options.walThreshold;
        }
        if (options.autoCheckpoint !== undefined) {
            engineOptions.autoCheckpoint = options.autoCheckpoint;
        }
        if (options.fsyncStrategy !== undefined) {
            engineOptions.fsyncStrategy = options.fsyncStrategy;
        }

        const storageEngine = await createStorageEngine(path, dbName, engineOptions);
        return new Database(storageEngine, dbName);
    }

    /**
     * 获取数据库名称
     */
    public get dbName(): string {
        return this.dbNameValue;
    }

    /**
     * 获取数据库路径
     */
    public get path(): string {
        return this.storageEngine.getBasePath();
    }

    /**
     * 检查数据库是否已打开
     */
    public get isOpen(): boolean {
        return !this.closed && this.storageEngine.isInitialized();
    }

    /**
     * 读取键值
     *
     * @param key 键
     * @returns 值，不存在时返回 `null`
     */
    public async get(key: string): Promise<Uint8Array | null> {
        this.ensureOpen();
        this.ensureUserKey(key);

        const value = await this.storageEngine.get(key);
        return value === null ? null : new Uint8Array(value);
    }

    /**
     * 写入键值
     *
     * @param key 键
     * @param value 值，字符串按 UTF-8 编码
     */
    public async put(key: string, value: Uint8Array | string): Promise<void> {
        this.ensureOpen();
        this.ensureUserKey(key);

        const bytes = typeof value === "string" ? new TextEncoder().encode(value) : value;
        await this.storageEngine.upsert(key, Buffer.from(bytes));
    }

    /**
     * 删除键值
     *
     * @param key 键
     * @returns 是否删除成功，键不存在时返回 `false`
     */
    public async delete(key: string): Promise<boolean> {
        this.ensureOpen();
        this.ensureUserKey(key);

        if (!(await this.storageEngine.exists(key))) {
            return false;
        }

        await this.storageEngine.delete(key);
        return true;
    }

    /**
     * 读取 Schema
     *
     * @returns Schema 版本与文档，未持久化时返回 `null`
     */
    public async getSchema(): Promise<SchemaVersion | null> {
        this.ensureOpen();

        const raw = await this.storageEngine.get(SCHEMA_RECORD_KEY);
        if (raw === null) {
            return null;
        }

        try {
            const record = JSON.parse(raw.toString("utf-8")) as SchemaRecord;
            return { version: record.version, document: record.document };
        } catch (error) {
            throw new LightDbError(LightDbErrorCodes.corrupt, "stored schema record is not valid JSON", {
                operation: "schemaGet",
                cause: error,
            });
        }
    }

    /**
     * 读取 Schema 版本
     *
     * @returns 版本号，未持久化时返回 `null`
     */
    public async schemaVersion(): Promise<number | null> {
        const schema = await this.getSchema();
        return schema === null ? null : schema.version;
    }

    /**
     * 首次使用时持久化 Schema，之后要求文档完全一致
     *
     * 与 Rust 引擎一致：文档里的 `// @yydb-schema-version:` 决定版本，
     * 线协议中的版本参数不参与判定，文档是唯一真相。
     *
     * @param _version 版本参数，保留以便与 `@yydb/yydb` 的签名同构
     * @param document Schema 文档
     */
    public async ensureSchema(_version: number, document: string): Promise<void> {
        this.ensureOpen();

        const current = await this.getSchema();
        if (current !== null) {
            if (current.document !== document) {
                throw new LightDbError(
                    LightDbErrorCodes.schemaConflict,
                    "schema document changed -- migrate the schema to advance the version",
                    { operation: "schemaEnsure" },
                );
            }
            return;
        }

        if (document.includes("\0")) {
            throw new LightDbError(LightDbErrorCodes.schemaInvalid, "schema document must not contain NUL bytes", {
                operation: "schemaEnsure",
            });
        }

        const record: SchemaRecord = { version: documentVersion(document), document };
        await this.storageEngine.upsert(SCHEMA_RECORD_KEY, Buffer.from(JSON.stringify(record), "utf-8"));
    }

    /**
     * 统计用户键数量，内部记录不计入
     */
    public async recordCount(): Promise<number> {
        this.ensureOpen();

        const keys = await this.storageEngine.getAllKeys();
        return keys.filter((key) => !Database.isReservedKey(key)).length;
    }

    /**
     * 执行检查点，将 WAL 中的写入合并到主文件
     */
    public async checkpoint(): Promise<CheckpointResult> {
        this.ensureOpen();
        return this.storageEngine.checkpoint();
    }

    /**
     * 获取存储统计信息
     */
    public async getStats(): Promise<StorageStats> {
        this.ensureOpen();
        return this.storageEngine.getStats();
    }

    /**
     * 关闭数据库并释放文件句柄
     */
    public async close(): Promise<void> {
        if (this.closed) {
            return;
        }

        await this.storageEngine.close();
        this.closed = true;
    }

    /**
     * 判断是否为内部保留键
     * @param key 键
     */
    private static isReservedKey(key: string): boolean {
        return RESERVED_KEY_PREFIXES.some((prefix) => key.startsWith(prefix));
    }

    /**
     * 拒绝写入内部键空间
     * @param key 键
     */
    private ensureUserKey(key: string): void {
        if (Database.isReservedKey(key)) {
            throw new LightDbError(LightDbErrorCodes.unsupported, `key "${key}" uses a reserved prefix`, {
                operation: "kv",
            });
        }
        if (key.length === 0) {
            throw new LightDbError(LightDbErrorCodes.unsupported, "key must not be empty", { operation: "kv" });
        }
    }

    /**
     * 确保数据库处于打开状态
     */
    private ensureOpen(): void {
        if (this.closed) {
            throw new LightDbError(LightDbErrorCodes.sessionClosed, "database is closed", {
                operation: "database",
            });
        }
        if (!this.storageEngine.isInitialized()) {
            throw new LightDbError(LightDbErrorCodes.io, "database is not initialized", {
                operation: "database",
            });
        }
    }
}
