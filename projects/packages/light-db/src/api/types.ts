/**
 * LightDB API 层类型定义
 * 包含所有公共 API 接口和类型定义
 */

import type { Collection } from "../collection";
import type { CollectionOptions, IndexConfig } from "../collection/types";
import type { StorageEngineOptions, StorageStats, CheckpointResult } from "../types";

/**
 * LightDB 配置选项
 */
export interface LightDBOptions extends StorageEngineOptions {
    /** 数据库名称，默认为 'default' */
    dbName?: string;
}

/**
 * 集合配置选项
 * 用于创建类型安全的集合
 */
export interface CollectionCreateOptions<T extends Record<string, unknown>> extends CollectionOptions<T> {
    /** 集合名称（必填） */
    name: string;
}

/**
 * 事务上下文
 * 提供事务内的集合访问
 */
export interface TransactionContext {
    /** 获取集合 */
    collection<T extends Record<string, unknown>>(name: string): TransactionCollection<T>;
}

/**
 * 事务集合接口
 * 提供事务内的集合操作
 */
export interface TransactionCollection<T extends Record<string, unknown>> {
    /** 查找文档 */
    find(filter?: Record<string, unknown>): Promise<T[]>;
    /** 查找单个文档 */
    findOne(filter: Record<string, unknown>): Promise<T | null>;
    /** 插入文档 */
    insert(doc: T): Promise<void>;
    /** 更新文档 */
    update(filter: Record<string, unknown>, update: Partial<T>): Promise<number>;
    /** 删除文档 */
    delete(filter: Record<string, unknown>): Promise<number>;
}

/**
 * 事务回调函数类型
 */
export type TransactionCallback = (tx: TransactionContext) => Promise<void>;

/**
 * 事务选项
 */
export interface TransactionOptions {
    /** 事务超时时间（毫秒），默认 5000 */
    timeout?: number;
    /** 事务隔离级别 */
    isolationLevel?: "read_committed" | "repeatable_read" | "serializable";
}

/**
 * 事务状态
 */
export type TransactionState = "active" | "committed" | "rolled_back" | "failed";

/**
 * 事务结果
 */
export interface TransactionResult {
    /** 事务是否成功 */
    success: boolean;
    /** 事务状态 */
    state: TransactionState;
    /** 事务持续时间（毫秒） */
    durationMs: number;
    /** 错误信息（如果失败） */
    error?: string;
}

/**
 * 数据库统计信息
 */
export interface DatabaseStats extends StorageStats {
    /** 数据库名称 */
    dbName: string;
    /** 集合数量 */
    collectionCount: number;
    /** 集合统计信息列表 */
    collections: Array<{
        name: string;
        documentCount: number;
        indexCount: number;
    }>;
}

/**
 * 数据库配置信息
 */
export interface DatabaseConfig {
    /** 数据库名称 */
    dbName: string;
    /** 数据库路径 */
    path: string;
    /** 页面大小 */
    pageSize: number;
    /** WAL 阈值 */
    walThreshold: number;
    /** 是否启用自动检查点 */
    autoCheckpoint: boolean;
}

/**
 * LightDB 实例接口
 * 定义 LightDB 的公共 API
 */
export interface ILightDB {
    /** 获取数据库名称 */
    readonly dbName: string;
    /** 获取数据库路径 */
    readonly path: string;
    /** 检查数据库是否已打开 */
    readonly isOpen: boolean;

    /** 获取或创建集合 */
    collection<T extends Record<string, unknown>>(name: string, options: CollectionCreateOptions<T>): Collection<T>;

    /** 检查集合是否存在 */
    hasCollection(name: string): boolean;

    /** 删除集合 */
    dropCollection(name: string): boolean;

    /** 获取所有集合名称 */
    getCollectionNames(): string[];

    /** 执行事务 */
    transaction(callback: TransactionCallback, options?: TransactionOptions): Promise<TransactionResult>;

    /** 执行检查点 */
    checkpoint(): Promise<CheckpointResult>;

    /** 获取数据库统计信息 */
    getStats(): Promise<DatabaseStats>;

    /** 获取数据库配置 */
    getConfig(): DatabaseConfig;

    /** 同步数据到磁盘 */
    sync(): Promise<void>;

    /** 关闭数据库 */
    close(): Promise<void>;
}

/**
 * 定义集合 Schema 的辅助类型
 */
export type DefineCollectionSchema<T extends Record<string, unknown>> = {
    /** 文档类型 */
    schema: T;
    /** 主键字段 */
    primaryKey: keyof T & string;
    /** 二级索引配置 */
    indexes?: IndexConfig<T>[];
};

/**
 * 创建集合 Schema 的辅助函数返回类型
 */
export type CollectionSchema<T extends Record<string, unknown>> = DefineCollectionSchema<T>;
