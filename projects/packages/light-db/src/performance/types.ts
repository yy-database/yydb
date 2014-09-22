/**
 * 性能优化模块类型定义
 * 包含列式存储、批量写入、读写锁等组件的类型定义
 */

/**
 * 列类型枚举
 */
export enum ColumnType {
    Int8 = "int8",
    Int16 = "int16",
    Int32 = "int32",
    Int64 = "int64",
    UInt8 = "uint8",
    UInt16 = "uint16",
    UInt32 = "uint32",
    UInt64 = "uint64",
    Float32 = "float32",
    Float64 = "float64",
    String = "string",
    Boolean = "boolean",
}

/**
 * 列定义接口
 */
export interface ColumnDefinition {
    /** 列名称 */
    name: string;
    /** 列类型 */
    type: ColumnType;
    /** 是否可为空 */
    nullable?: boolean;
}

/**
 * 列式存储配置
 */
export interface ColumnarStorageOptions {
    /** 列定义列表 */
    columns: ColumnDefinition[];
    /** 初始容量 */
    initialCapacity?: number;
    /** 是否自动扩容 */
    autoGrow?: boolean;
}

/**
 * 列式存储统计信息
 */
export interface ColumnarStorageStats {
    /** 列数量 */
    columnCount: number;
    /** 行数量 */
    rowCount: number;
    /** 内存使用量（字节） */
    memoryUsage: number;
    /** 列统计信息 */
    columns: ColumnStats[];
}

/**
 * 列统计信息
 */
export interface ColumnStats {
    /** 列名称 */
    name: string;
    /** 列类型 */
    type: ColumnType;
    /** 非空值数量 */
    nonNullCount: number;
    /** 内存使用量（字节） */
    memoryUsage: number;
}

/**
 * 批量写入配置
 */
export interface BatchWriteOptions {
    /** 批量大小阈值（记录数） */
    batchSize?: number;
    /** 刷新间隔（毫秒） */
    flushIntervalMs?: number;
    /** fsync 策略 */
    fsyncStrategy?: FsyncStrategyType;
    /** fsync 批量大小 */
    fsyncBatchSize?: number;
    /** fsync 间隔（毫秒） */
    fsyncIntervalMs?: number;
}

/**
 * fsync 策略类型
 */
export type FsyncStrategyType = "always" | "batch" | "interval" | "manual";

/**
 * 批量写入统计信息
 */
export interface BatchWriteStats {
    /** 待写入记录数 */
    pendingCount: number;
    /** 已写入记录数 */
    writtenCount: number;
    /** 上次刷新时间 */
    lastFlushTime: number;
    /** 上次 fsync 时间 */
    lastFsyncTime: number;
    /** 队列容量 */
    queueCapacity: number;
}

/**
 * 批量写入结果
 */
export interface BatchFlushResult {
    /** 成功写入的记录数 */
    recordsWritten: number;
    /** 写入字节数 */
    bytesWritten: number;
    /** 刷新耗时（毫秒） */
    durationMs: number;
    /** 是否执行了 fsync */
    fsynced: boolean;
}

/**
 * 读写锁配置
 */
export interface RWLockOptions {
    /** 最大等待时间（毫秒），0 表示无限等待 */
    timeout?: number;
    /** 是否启用公平模式 */
    fair?: boolean;
}

/**
 * 读写锁统计信息
 */
export interface RWLockStats {
    /** 当前读锁持有者数量 */
    readLockCount: number;
    /** 当前写锁持有者数量（0 或 1） */
    writeLockCount: number;
    /** 等待读锁的数量 */
    pendingReadCount: number;
    /** 等待写锁的数量 */
    pendingWriteCount: number;
    /** 总读锁获取次数 */
    totalReadAcquires: number;
    /** 总写锁获取次数 */
    totalWriteAcquires: number;
}

/**
 * 惰性查询配置
 */
export interface LazyQueryOptions {
    /** 是否启用索引优化 */
    useIndex?: boolean;
    /** 预取大小 */
    prefetchSize?: number;
}

/**
 * 惰性查询统计信息
 */
export interface LazyQueryStats {
    /** 已迭代元素数量 */
    iteratedCount: number;
    /** 是否已完成 */
    completed: boolean;
    /** 是否使用了索引 */
    usedIndex: boolean;
    /** 索引名称 */
    indexName?: string;
}

/**
 * 聚合操作类型
 */
export type AggregationType = "sum" | "avg" | "min" | "max" | "count" | "countDistinct";

/**
 * 聚合结果
 */
export interface AggregationResult {
    /** 聚合类型 */
    type: AggregationType;
    /** 字段名称 */
    field: string;
    /** 聚合值 */
    value: number | bigint | null;
    /** 计算的记录数 */
    recordCount: number;
}

/**
 * 范围查询条件
 */
export interface RangeCondition<T = unknown> {
    /** 大于 */
    gt?: T;
    /** 大于等于 */
    gte?: T;
    /** 小于 */
    lt?: T;
    /** 小于等于 */
    lte?: T;
}

/**
 * Buffer 池配置
 */
export interface BufferPoolOptions {
    /** 池大小 */
    poolSize?: number;
    /** 块大小 */
    chunkSize?: number;
    /** 是否启用预分配 */
    preallocate?: boolean;
}

/**
 * Buffer 池统计信息
 */
export interface BufferPoolStats {
    /** 总块数 */
    totalChunks: number;
    /** 可用块数 */
    availableChunks: number;
    /** 已使用块数 */
    usedChunks: number;
    /** 总内存（字节） */
    totalMemory: number;
    /** 使用内存（字节） */
    usedMemory: number;
}

/**
 * 性能优化模块配置
 */
export interface PerformanceOptions {
    /** 列式存储配置 */
    columnarStorage?: ColumnarStorageOptions;
    /** 批量写入配置 */
    batchWrite?: BatchWriteOptions;
    /** 读写锁配置 */
    rwLock?: RWLockOptions;
    /** Buffer 池配置 */
    bufferPool?: BufferPoolOptions;
}

/**
 * 性能优化统计信息
 */
export interface PerformanceStats {
    /** 列式存储统计 */
    columnarStorage?: ColumnarStorageStats;
    /** 批量写入统计 */
    batchWrite?: BatchWriteStats;
    /** 读写锁统计 */
    rwLock?: RWLockStats;
    /** Buffer 池统计 */
    bufferPool?: BufferPoolStats;
}
