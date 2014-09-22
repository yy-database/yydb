/**
 * LightDB 存储引擎
 *
 * 这是一个高性能的 TypeScript 数据库存储层实现。
 * 提供完整的 WAL（预写日志）、检查点、崩溃恢复和索引管理功能。
 *
 * @packageDocumentation
 */

// 常量导出
export {
    LIGHT_MAGIC,
    LIGHT_MAGIC_STRING,
    LIGHT_VERSION,
    DEFAULT_PAGE_SIZE,
    DEFAULT_WAL_THRESHOLD,
    HEADER_SIZE,
    WAL_RECORD_HEADER_SIZE,
    SHM_HEADER_SIZE,
    SHM_ENTRY_SIZE,
    SHM_DEFAULT_CAPACITY,
    PAGE_HEADER_SIZE,
    OperationType,
    FILE_EXTENSIONS,
} from "./constants";

// 类型导出
export {
    LightHeader,
    Record,
    Page,
    WalRecord,
    ShmHeader,
    ShmEntry,
    StorageEngineOptions,
    FsyncStrategy,
    CheckpointResult,
    RecoveryResult,
    StorageStats,
    WriteResult,
    BatchWriteResult,
    HashIteratorResult,
    FileHandle,
    PageMap,
    RecordMap,
    WalRecordIterator,
} from "./types";

// 错误类导出
export {
    StorageError,
    FileOperationError,
    FileFormatError,
    ChecksumError,
    WalError,
    RecoveryError,
    CheckpointError,
    PageError,
    SerializationError,
    ConfigurationError,
} from "./errors";

// 工具类导出
export {
    BinaryUtils,
    BufferWriter,
    BufferReader,
    FileUtils,
    FileHandleWrapper,
} from "./utils";

// 存储组件导出
export {
    LightFileSerializer,
    LightFileManager,
    PageManager,
} from "./storage";

export {
    WalRecordSerializer,
    WalWriter,
    WalReader,
    WalManager,
} from "./storage";

export {
    ShmSerializer,
    ShmHashTable,
    ShmFileManager,
    WalIndexManager,
} from "./storage";

export {
    CheckpointManager,
    AutoCheckpointScheduler,
    IncrementalCheckpointManager,
    CheckpointOptions,
} from "./storage";

export {
    RecoveryManager,
    FastRecoveryManager,
    RecoveryValidator,
    RecoveryPhase,
    RecoveryContext,
} from "./storage";

export {
    StorageEngine,
    createStorageEngine,
} from "./storage";

import type { StorageEngineOptions } from "./types";
import { createStorageEngine as createEngine, StorageEngine as StorageEngineClass } from "./storage";

/**
 * 打开存储引擎
 *
 * @param basePath - 数据库文件存储路径
 * @param dbName - 数据库名称
 * @param options - 配置选项
 * @returns 初始化完成的存储引擎实例
 *
 * @example
 * ```typescript
 * import { openStorageEngine } from '@yydb/light-db';
 *
 * const engine = await openStorageEngine('./data', 'mydb', {
 *   pageSize: 4096,
 *   walThreshold: 16 * 1024 * 1024,
 *   autoCheckpoint: true
 * });
 *
 * // 插入数据
 * await engine.insert('key1', Buffer.from('value1'));
 *
 * // 读取数据
 * const value = await engine.get('key1');
 *
 * // 关闭引擎
 * await engine.close();
 * ```
 */
export async function openStorageEngine(basePath: string, dbName: string, options?: StorageEngineOptions): Promise<StorageEngineClass> {
    return createEngine(basePath, dbName, options);
}

// 索引引擎导出
export {
    IndexKey,
    IndexValue,
    RangeCondition,
    EqualityCondition,
    QueryCondition,
    IndexType,
    IndexOptions,
    PrimaryIndexOptions,
    SecondaryIndexOptions,
    IndexStats,
    IndexQueryResult,
    BPlusTreeNodeType,
    BPlusTreeNode,
    BPlusTreeLeafNode,
    BPlusTreeInternalNode,
    IndexOperationResult,
    IndexBatchResult,
    IndexIteratorOptions,
    IndexEntry,
    IndexChangeRecord,
    IndexManagerOptions,
    Comparator,
    KeyExtractor,
    ValueExtractor,
} from "./indexes";

export {
    IndexError,
    DuplicateKeyError,
    KeyNotFoundError,
    IndexNotFoundError,
    IndexExistsError,
    IndexConfigError,
    IndexOperationError,
    IndexConsistencyError,
    RangeQueryError,
} from "./indexes";

export { BPlusTree } from "./indexes";
export { PrimaryIndex, PrimaryIndexConfig } from "./indexes";
export { SecondaryIndex, SecondaryIndexConfig } from "./indexes";
export { IndexManager, CollectionIndexConfig } from "./indexes";

// 集合层导出
export {
    PrimaryKeyType,
    Document,
    ComparisonOperators,
    ArrayOperators,
    ExistenceOperators,
    RegexOperators,
    FieldCondition,
    LogicalOperators,
    QueryFilter,
    UpdateOperators,
    UpdateOperation,
    SortDirection,
    SortCondition,
    QueryOptions,
    InsertOptions,
    UpdateOptions,
    DeleteOptions,
    InsertResult,
    InsertManyResult,
    UpdateResult,
    DeleteResult,
    FindResult,
    IndexConfig,
    CollectionOptions,
    CollectionStats,
    CollectionEventType,
    CollectionEventListener,
    CursorOptions,
    AggregationStage,
} from "./collection";

export { QueryParser, QueryParseResult } from "./collection";
export { QueryBuilder, QueryExecutor } from "./collection";
export { Collection, CollectionError } from "./collection";

// API 层导出
export {
    LightDBOptions,
    CollectionCreateOptions,
    TransactionContext,
    TransactionCollection,
    TransactionCallback,
    TransactionOptions,
    TransactionState,
    TransactionResult,
    DatabaseStats,
    DatabaseConfig,
    ILightDB,
    DefineCollectionSchema,
    CollectionSchema,
} from "./api";

export { TransactionError, Transaction, TransactionManager } from "./api";

export {
    LightDB,
    defineCollectionSchema,
    openDatabase,
} from "./api";

// 性能优化模块导出
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
    RangeCondition as PerformanceRangeCondition,
    BufferPoolOptions,
    BufferPoolStats,
    PerformanceOptions,
    PerformanceStats,
} from "./performance";

export { ColumnarStorage } from "./performance";

export {
    BufferPool,
    getGlobalBufferPool,
    OptimizedBufferWriter,
    OptimizedBufferReader,
    FastSerializer,
    BufferConcatenator,
} from "./performance";

export {
    WalRecordEntry,
    BatchWalWriter,
    BatchWriteManager,
} from "./performance";

export {
    ReadLockHandle,
    WriteLockHandle,
    RWLock,
    RWLockManager,
    ReentrantRWLock,
} from "./performance";

export {
    LazyQueryIterator,
    LazyQueryBuilder,
    lazy,
    lazyIterator,
    GeneratorUtils,
} from "./performance";
