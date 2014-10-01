/**
 * 浏览器数据库作者面类型
 *
 * 这些类型是 `@yydb/light-db` 的稳定合同：二进制一律 `Uint8Array`，
 * 不出现本地路径、文件句柄或 Node 专属类型。
 */

import type { LeaseMode } from "../core/lease";
import type { CheckpointResult, StorageEngineStats } from "../core/engine";
import type { StorageAdapterFactory } from "../storage/StorageAdapter";

/** Schema 版本与文档，与 `@yydb/yydb` 同构 */
export interface SchemaVersion {
    /** Schema 版本 */
    version: number;
    /** Schema 文档全文 */
    document: string;
}

/** 打开数据库的选项 */
export interface LightDatabaseOptions {
    /** 页大小，默认 4KB */
    pageSize?: number;
    /** WAL 字节阈值，超过后自动检查点，默认 16MB */
    walThreshold?: number;
    /** 是否自动检查点，默认 true */
    autoCheckpoint?: boolean;
    /** 写租约模式，默认 `web-locks` */
    lease?: LeaseMode;
    /** 存储后端，默认 OPFS */
    storage?: StorageAdapterFactory;
}

/**
 * 浏览器数据库
 *
 * 与 `@yydb/yydb` 的 TypeScript 作者面保持同一调用形态与返回语义。
 */
export interface LightDatabase {
    /** 数据库名 */
    readonly name: string;

    /** 读取已持久化的 schema */
    schema(): Promise<SchemaVersion | null>;

    /** 声明 schema 文档，文档是唯一真相 */
    ensureSchema(document: string): Promise<void>;

    /** 读取键 */
    get(key: string): Promise<Uint8Array | null>;

    /** 写入键 */
    put(key: string, value: Uint8Array): Promise<void>;

    /** 删除键，键存在时返回 true */
    delete(key: string): Promise<boolean>;

    /** 开启事务 */
    begin(): Promise<void>;

    /** 提交事务 */
    commit(): Promise<void>;

    /** 回滚事务 */
    rollback(): Promise<void>;

    /** 执行检查点 */
    checkpoint(): Promise<CheckpointResult>;

    /** 数据库统计 */
    stats(): StorageEngineStats;

    /** 关闭数据库 */
    close(): Promise<void>;
}
