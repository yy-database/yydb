/**
 * 性能优化模块
 * 提供列式存储、二进制序列化优化、异步批量写入、读写锁和惰性求值查询
 *
 * @packageDocumentation
 */

// 类型导出
export {
    ColumnType,
    ColumnDefinition,
    ColumnarStorageOptions,
    ColumnarStorageStats,
    ColumnStats,
    BatchWriteOptions,
    BatchWriteStats,
    BatchFlushResult,
    FsyncStrategyType,
    RWLockOptions,
    RWLockStats,
    LazyQueryOptions,
    LazyQueryStats,
    AggregationType,
    AggregationResult,
    RangeCondition,
    BufferPoolOptions,
    BufferPoolStats,
    PerformanceOptions,
    PerformanceStats,
} from "./types";

// 列式存储导出
export { ColumnarStorage } from "./ColumnarStorage";

// 二进制序列化优化导出
export {
    BufferPool,
    getGlobalBufferPool,
    OptimizedBufferWriter,
    OptimizedBufferReader,
    FastSerializer,
    BufferConcatenator,
} from "./OptimizedBuffer";

// 异步批量写入导出
export {
    WalRecordEntry,
    BatchWalWriter,
    BatchWriteManager,
} from "./BatchWalWriter";

// 读写锁导出
export {
    ReadLockHandle,
    WriteLockHandle,
    RWLock,
    RWLockManager,
    ReentrantRWLock,
} from "./RWLock";

// 惰性求值查询导出
export {
    LazyQueryIterator,
    LazyQueryBuilder,
    lazy,
    lazyIterator,
    GeneratorUtils,
} from "./LazyQuery";
