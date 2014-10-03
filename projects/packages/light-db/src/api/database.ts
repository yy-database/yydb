/**
 * 浏览器数据库
 *
 * `@yydb/light-db` 的公开入口。调用形态与 `@yydb/yydb` 的 TypeScript 作者面一致，
 * 但只暴露浏览器可用的类型：数据库名、`Uint8Array`、结构化错误码。
 */

import { StorageEngine } from "../core/engine";
import type { CheckpointResult, StorageEngineOptions, StorageEngineStats } from "../core/engine";
import { LightDbError, LightDbErrorCodes } from "../errors";
import { isOpfsAvailable, OpfsStorageAdapterFactory } from "../storage/opfs/OpfsStorageAdapter";
import type { StorageAdapterFactory } from "../storage/StorageAdapter";
import {
    decodeSchema,
    encodeSchema,
    parseSchemaDocument,
    RESERVED_KEY_PREFIX,
    SCHEMA_KEY,
} from "./schema";
import { Transaction } from "./transaction";
import type { LightDatabase, LightDatabaseOptions, SchemaVersion } from "./types";

/** 数据库名最大长度 */
const MAX_DATABASE_NAME_LENGTH = 128;

/**
 * 浏览器数据库实现
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
            throw new LightDbError(
                LightDbErrorCodes.unsupported,
                `database name must be 1-${MAX_DATABASE_NAME_LENGTH} characters`,
                { operation: "open" },
            );
        }

        const storage = resolveStorageFactory(options);
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
        if (options.lease !== undefined) {
            engineOptions.lease = options.lease;
        }

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
     * @param document Schema 文档
     */
    public async ensureSchema(document: string): Promise<void> {
        const version = parseSchemaDocument(document);
        const existing = await this.schema();

        if (existing) {
            if (existing.document !== document) {
                throw new LightDbError(
                    LightDbErrorCodes.schemaConflict,
                    `schema document conflict: stored version ${existing.version}, incoming version ${version}`,
                    { operation: "ensureSchema" },
                );
            }
            return;
        }

        await this.engine.put(SCHEMA_KEY, encodeSchema({ version, document }));
        this.engine.notifySchemaChange();
    }

    /**
     * 读取键
     * @param key 键
     */
    public async get(key: string): Promise<Uint8Array | null> {
        return this.engine.get(this.assertUserKey(key, "kvGet"));
    }

    /**
     * 写入键
     * @param key 键
     * @param value 值
     */
    public async put(key: string, value: Uint8Array): Promise<void> {
        await this.engine.put(this.assertUserKey(key, "kvPut"), value);
    }

    /**
     * 删除键
     * @param key 键
     */
    public async delete(key: string): Promise<boolean> {
        return this.engine.delete(this.assertUserKey(key, "kvDelete"));
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
     * @param key 键
     * @param operation 操作名
     */
    private assertUserKey(key: string, operation: string): string {
        if (key.length === 0) {
            throw new LightDbError(LightDbErrorCodes.unsupported, "key must not be empty", { operation });
        }
        if (key.startsWith(RESERVED_KEY_PREFIX)) {
            throw new LightDbError(
                LightDbErrorCodes.unsupported,
                `key prefix "${RESERVED_KEY_PREFIX}" is reserved for the database`,
                { operation },
            );
        }
        return key;
    }
}

/**
 * 打开浏览器数据库
 * @param name 数据库名
 * @param options 打开选项
 */
export async function openLightDatabase(name: string, options: LightDatabaseOptions = {}): Promise<LightDatabase> {
    return BrowserDatabase.open(name, options);
}

/**
 * 解析存储后端
 * @param options 打开选项
 */
function resolveStorageFactory(options: LightDatabaseOptions): StorageAdapterFactory {
    if (options.storage) {
        return options.storage;
    }

    if (!isOpfsAvailable()) {
        throw new LightDbError(LightDbErrorCodes.unsupported, "OPFS is unavailable, pass an explicit storage adapter", {
            operation: "open",
        });
    }

    return new OpfsStorageAdapterFactory();
}
