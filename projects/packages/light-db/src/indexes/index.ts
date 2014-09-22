/**
 * LightDB 索引引擎
 *
 * 提供高效的索引管理功能，包括：
 * - B+ 树主键索引
 * - 二级索引（Map + Set 结构）
 * - 索引管理器
 *
 * @packageDocumentation
 */

// 类型导出
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
} from "./types";

// 错误类导出
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
} from "./errors";

// B+ 树导出
export { BPlusTree } from "./BPlusTree";

// 主键索引导出
export { PrimaryIndex, PrimaryIndexConfig } from "./PrimaryIndex";

// 二级索引导出
export { SecondaryIndex, SecondaryIndexConfig } from "./SecondaryIndex";

// 索引管理器导出
export { IndexManager, CollectionIndexConfig } from "./IndexManager";
