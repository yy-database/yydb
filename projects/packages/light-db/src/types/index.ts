/**
 * LightDB 存储引擎类型定义
 * 包含所有公共接口、类型和结构体定义
 */

import { OperationType } from "../constants";

/**
 * .light 文件头部结构
 * 存储文件元信息和全局状态
 */
export interface LightHeader {
    /** 魔数：用于验证文件格式 */
    magic: number;
    /** 文件格式版本号 */
    version: number;
    /** 页面大小（字节） */
    pageSize: number;
    /** 上次快照时的 LSN */
    lastSnapshotLsn: bigint;
}

/**
 * 数据记录结构
 * 表示存储在页面中的单条记录
 */
export interface Record {
    /** 记录键 */
    key: string;
    /** 记录值（序列化后的二进制数据） */
    value: Buffer;
}

/**
 * 页面结构
 * 表示 .light 文件中的一个数据页
 */
export interface Page {
    /** 页面ID */
    pageId: number;
    /** 页面中的记录列表 */
    records: Record[];
    /** 页面校验和 */
    checksum: number;
}

/**
 * WAL 记录结构
 * 表示预写日志中的一条记录
 */
export interface WalRecord {
    /** 日志序列号，单调递增 */
    lsn: bigint;
    /** 事务ID */
    transactionId: bigint;
    /** 操作类型 */
    operationType: OperationType;
    /** 操作的键 */
    key: string;
    /** 操作的值（可选，删除操作无值） */
    value: Buffer | null;
    /** 记录校验和 */
    checksum: number;
}

/**
 * SHM 文件头部结构
 * 存储 WAL 索引的元信息
 */
export interface ShmHeader {
    /** 当前 WAL 文件大小 */
    walFileSize: bigint;
    /** 最后一个完整页的 LSN */
    lastCompleteLsn: bigint;
    /** 哈希表中记录数量 */
    recordCount: number;
    /** 头部校验和 */
    checksum: number;
}

/**
 * SHM 哈希表条目
 * 用于快速定位 WAL 中每个页面的最新记录
 */
export interface ShmEntry {
    /** 页面ID */
    pageId: number;
    /** 该页面最新记录的 LSN */
    lsn: bigint;
    /** 该记录在 WAL 文件中的偏移量 */
    walOffset: bigint;
}

/**
 * 存储引擎配置选项
 */
export interface StorageEngineOptions {
    /** 页面大小（字节），默认 4KB */
    pageSize?: number;
    /** WAL 文件大小阈值，超过此值自动触发检查点，默认 16MB */
    walThreshold?: number;
    /** 是否启用自动检查点，默认 true */
    autoCheckpoint?: boolean;
    /** fsync 策略：每次写入、每N条、或每N毫秒 */
    fsyncStrategy?: FsyncStrategy;
}

/**
 * fsync 策略配置
 */
export interface FsyncStrategy {
    /** 策略类型 */
    type: "always" | "batch" | "interval";
    /** 批量模式下，每N条记录 fsync 一次 */
    batchSize?: number;
    /** 间隔模式下，每N毫秒 fsync 一次 */
    intervalMs?: number;
}

/**
 * 检查点结果
 */
export interface CheckpointResult {
    /** 检查点完成时间 */
    timestamp: number;
    /** 检查点时的 LSN */
    lsn: bigint;
    /** 写入的页面数量 */
    pagesWritten: number;
    /** 写入的总字节数 */
    bytesWritten: number;
}

/**
 * 恢复结果
 */
export interface RecoveryResult {
    /** 恢复是否成功 */
    success: boolean;
    /** 回放的 WAL 记录数量 */
    recordsReplayed: number;
    /** 加载的页面数量 */
    pagesLoaded: number;
    /** 恢复耗时（毫秒） */
    durationMs: number;
    /** 错误信息（如果恢复失败） */
    error?: string;
}

/**
 * 存储引擎统计信息
 */
export interface StorageStats {
    /** 当前 LSN */
    currentLsn: bigint;
    /** 当前事务ID */
    currentTransactionId: bigint;
    /** WAL 文件大小 */
    walFileSize: number;
    /** .light 文件大小 */
    lightFileSize: number;
    /** 页面数量 */
    pageCount: number;
    /** 记录数量 */
    recordCount: number;
    /** 上次检查点时间 */
    lastCheckpointTime: number | null;
}

/**
 * 写操作结果
 */
export interface WriteResult {
    /** 操作是否成功 */
    success: boolean;
    /** 分配的 LSN */
    lsn: bigint;
    /** 写入的字节数 */
    bytesWritten: number;
}

/**
 * 批量写操作结果
 */
export interface BatchWriteResult {
    /** 操作是否成功 */
    success: boolean;
    /** 起始 LSN */
    startLsn: bigint;
    /** 结束 LSN */
    endLsn: bigint;
    /** 总写入字节数 */
    totalBytesWritten: number;
    /** 成功的记录数量 */
    recordsWritten: number;
}

/**
 * 哈希表迭代器结果
 */
export interface HashIteratorResult {
    /** 条目数据 */
    entry: ShmEntry;
    /** 是否还有更多条目 */
    done: boolean;
}

/**
 * 文件句柄包装接口
 * 提供统一的文件操作抽象
 */
export interface FileHandle {
    /** 读取文件内容 */
    read(buffer: Buffer, offset: number, length: number, position: number): Promise<number>;
    /** 写入文件内容 */
    write(buffer: Buffer, offset: number, length: number, position: number): Promise<number>;
    /** 同步文件到磁盘 */
    sync(): Promise<void>;
    /** 关闭文件句柄 */
    close(): Promise<void>;
    /** 获取文件大小 */
    size(): Promise<number>;
}

/**
 * 内存中的数据页映射
 */
export type PageMap = Map<number, Page>;

/**
 * 内存中的记录映射
 */
export type RecordMap = Map<string, Buffer>;

/**
 * WAL 记录迭代器
 */
export type WalRecordIterator = AsyncGenerator<WalRecord, void, unknown>;
