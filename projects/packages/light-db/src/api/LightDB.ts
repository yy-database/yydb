/**
 * LightDB 主类
 * 提供用户直接使用的接口层
 */

import { StorageEngine, createStorageEngine } from "../storage";
import { Collection } from "../collection";
import type { CollectionOptions } from "../collection/types";
import { TransactionManager } from "./Transaction";
import type {
    LightDBOptions,
    CollectionCreateOptions,
    TransactionCallback,
    TransactionOptions,
    TransactionResult,
    DatabaseStats,
    DatabaseConfig,
    ILightDB,
    CollectionSchema,
} from "./types";
import type { CheckpointResult, StorageStats, StorageEngineOptions } from "../types";
import { StorageError } from "../errors";

/**
 * LightDB 数据库实例
 * 提供完整的数据库操作 API
 */
export class LightDB implements ILightDB {
    private readonly storageEngine: StorageEngine;
    private readonly collections: Map<string, Collection<Record<string, unknown>>>;
    private readonly transactionManager: TransactionManager;
    private readonly options: Required<Omit<LightDBOptions, "fsyncStrategy">> & {
        fsyncStrategy?: LightDBOptions["fsyncStrategy"];
    };
    private closed: boolean;

    /**
     * 创建 LightDB 实例
     * @param storageEngine 已初始化的存储引擎
     * @param options 配置选项
     */
    private constructor(
        storageEngine: StorageEngine,
        options: Required<Omit<LightDBOptions, "fsyncStrategy">> & {
            fsyncStrategy?: LightDBOptions["fsyncStrategy"];
        },
    ) {
        this.storageEngine = storageEngine;
        this.options = options;
        this.collections = new Map();
        this.transactionManager = new TransactionManager(this.collections as Map<string, Collection<Record<string, unknown>>>);
        this.closed = false;
    }

    /**
     * 打开数据库
     * 创建或打开指定路径的数据库
     *
     * @param path 数据库文件存储路径
     * @param options 配置选项
     * @returns 初始化完成的 LightDB 实例
     *
     * @example
     * ```typescript
     * const db = await LightDB.open('./data/mydb', {
     *   dbName: 'mydb',
     *   autoCheckpoint: true,
     *   pageSize: 4096
     * });
     * ```
     */
    public static async open(path: string, options: LightDBOptions = {}): Promise<LightDB> {
        const dbName = options.dbName ?? "default";
        const fullOptions = {
            dbName,
            pageSize: options.pageSize ?? 4096,
            walThreshold: options.walThreshold ?? 16 * 1024 * 1024,
            autoCheckpoint: options.autoCheckpoint ?? true,
            fsyncStrategy: options.fsyncStrategy,
        };

        const storageEngineOptions: StorageEngineOptions = {
            pageSize: fullOptions.pageSize,
            walThreshold: fullOptions.walThreshold,
            autoCheckpoint: fullOptions.autoCheckpoint,
        };

        if (fullOptions.fsyncStrategy !== undefined) {
            (storageEngineOptions as { fsyncStrategy?: typeof fullOptions.fsyncStrategy }).fsyncStrategy = fullOptions.fsyncStrategy;
        }

        const storageEngine = await createStorageEngine(path, dbName, storageEngineOptions);

        return new LightDB(storageEngine, fullOptions);
    }

    /**
     * 获取数据库名称
     */
    public get dbName(): string {
        return this.options.dbName;
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
     * 获取或创建集合
     * 返回类型安全的集合实例
     *
     * @param name 集合名称
     * @param options 集合配置选项
     * @returns 集合实例
     *
     * @example
     * ```typescript
     * interface User {
     *   id: number;
     *   name: string;
     *   age: number;
     * }
     *
     * const users = db.collection<User>('users', {
     *   name: 'users',
     *   primaryKey: 'id',
     *   indexes: [
     *     { name: 'name_idx', fieldName: 'name' }
     *   ]
     * });
     * ```
     */
    public collection<T extends Record<string, unknown>>(name: string, options: CollectionCreateOptions<T>): Collection<T> {
        this.ensureOpen();

        const existingCollection = this.collections.get(name);
        if (existingCollection) {
            return existingCollection as Collection<T>;
        }

        const collectionOptions: CollectionOptions<T> = {
            name,
            primaryKey: options.primaryKey,
            indexes: options.indexes ?? [],
        };

        const collection = new Collection<T>(collectionOptions);
        this.collections.set(name, collection as Collection<Record<string, unknown>>);

        return collection;
    }

    /**
     * 检查集合是否存在
     *
     * @param name 集合名称
     * @returns 集合是否存在
     */
    public hasCollection(name: string): boolean {
        this.ensureOpen();
        return this.collections.has(name);
    }

    /**
     * 删除集合
     *
     * @param name 集合名称
     * @returns 是否删除成功
     */
    public dropCollection(name: string): boolean {
        this.ensureOpen();

        const collection = this.collections.get(name);
        if (!collection) {
            return false;
        }

        collection.clear();
        this.collections.delete(name);
        return true;
    }

    /**
     * 获取所有集合名称
     *
     * @returns 集合名称数组
     */
    public getCollectionNames(): string[] {
        this.ensureOpen();
        return Array.from(this.collections.keys());
    }

    /**
     * 执行事务
     * 支持 ACID 事务特性
     *
     * @param callback 事务回调函数
     * @param options 事务选项
     * @returns 事务执行结果
     *
     * @example
     * ```typescript
     * await db.transaction(async (tx) => {
     *   const users = tx.collection<User>('users');
     *   const user = await users.findOne({ id: 1 });
     *   await users.update({ id: 1 }, { balance: user.balance - 100 });
     * });
     * ```
     */
    public async transaction(callback: TransactionCallback, options?: TransactionOptions): Promise<TransactionResult> {
        this.ensureOpen();
        return this.transactionManager.execute(callback, options);
    }

    /**
     * 执行检查点
     * 将内存中的数据持久化到磁盘
     *
     * @returns 检查点执行结果
     */
    public async checkpoint(): Promise<CheckpointResult> {
        this.ensureOpen();
        return this.storageEngine.checkpoint();
    }

    /**
     * 获取数据库统计信息
     *
     * @returns 数据库统计信息
     */
    public async getStats(): Promise<DatabaseStats> {
        this.ensureOpen();

        const storageStats: StorageStats = await this.storageEngine.getStats();

        const collectionStats = Array.from(this.collections.entries()).map(([name, collection]) => ({
            name,
            documentCount: collection.Size,
            indexCount: collection.getStats().indexCount,
        }));

        return {
            ...storageStats,
            dbName: this.options.dbName,
            collectionCount: this.collections.size,
            collections: collectionStats,
        };
    }

    /**
     * 获取数据库配置
     *
     * @returns 数据库配置信息
     */
    public getConfig(): DatabaseConfig {
        return {
            dbName: this.options.dbName,
            path: this.storageEngine.getBasePath(),
            pageSize: this.options.pageSize,
            walThreshold: this.options.walThreshold,
            autoCheckpoint: this.options.autoCheckpoint,
        };
    }

    /**
     * 同步数据到磁盘
     * 确保所有数据都已持久化
     */
    public async sync(): Promise<void> {
        this.ensureOpen();
        await this.storageEngine.sync();
    }

    /**
     * 关闭数据库
     * 释放所有资源并确保数据持久化
     */
    public async close(): Promise<void> {
        if (this.closed) {
            return;
        }

        for (const collection of this.collections.values()) {
            collection.clear();
        }
        this.collections.clear();

        await this.storageEngine.close();
        this.closed = true;
    }

    /**
     * 确保数据库处于打开状态
     */
    private ensureOpen(): void {
        if (this.closed) {
            throw new StorageError("Database is closed", "DATABASE_CLOSED");
        }
        if (!this.storageEngine.isInitialized()) {
            throw new StorageError("Database is not initialized", "DATABASE_NOT_INITIALIZED");
        }
    }

    /**
     * 获取存储引擎（内部使用）
     */
    public getStorageEngine(): StorageEngine {
        return this.storageEngine;
    }
}

/**
 * 定义集合 Schema 的辅助函数
 * 提供类型推断和 IDE 支持
 *
 * @param schema Schema 定义
 * @returns Schema 对象
 *
 * @example
 * ```typescript
 * const UserSchema = defineCollectionSchema({
 *   schema: {} as User,
 *   primaryKey: 'id',
 *   indexes: [
 *     { name: 'name_idx', fieldName: 'name' }
 *   ]
 * });
 * ```
 */
export function defineCollectionSchema<T extends Record<string, unknown>>(schema: CollectionSchema<T>): CollectionSchema<T> {
    return schema;
}

/**
 * 打开数据库的便捷函数
 *
 * @param path 数据库文件存储路径
 * @param options 配置选项
 * @returns 初始化完成的 LightDB 实例
 */
export async function openDatabase(path: string, options?: LightDBOptions): Promise<LightDB> {
    return LightDB.open(path, options);
}
