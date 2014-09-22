/**
 * LightDB 索引引擎类型定义
 * 包含所有索引相关的接口、类型和结构体定义
 */

/**
 * 索引键类型
 * 支持字符串和数值类型的主键
 */
export type IndexKey = string | number;

/**
 * 索引值类型
 * 二级索引的值类型
 */
export type IndexValue = string | number | boolean | null;

/**
 * 范围查询条件
 */
export interface RangeCondition<T = IndexKey> {
    /** 大于 */
    $gt?: T;
    /** 大于等于 */
    $gte?: T;
    /** 小于 */
    $lt?: T;
    /** 小于等于 */
    $lte?: T;
}

/**
 * 等值查询条件
 */
export interface EqualityCondition<T = IndexValue> {
    /** 等于 */
    $eq?: T;
    /** 不等于 */
    $ne?: T;
    /** 包含于 */
    $in?: T[];
    /** 不包含于 */
    $nin?: T[];
}

/**
 * 查询条件类型
 */
export type QueryCondition<T = IndexKey> = T | RangeCondition<T> | EqualityCondition<T>;

/**
 * 索引类型枚举
 */
export enum IndexType {
    /** 主键索引（B+树） */
    Primary = "primary",
    /** 二级索引（Map + Set） */
    Secondary = "secondary",
    /** 唯一索引 */
    Unique = "unique",
}

/**
 * 索引配置选项
 */
export interface IndexOptions {
    /** 索引名称 */
    name: string;
    /** 索引字段名 */
    fieldName: string;
    /** 索引类型 */
    type: IndexType;
    /** 是否唯一索引 */
    unique?: boolean;
    /** 是否稀疏索引（不索引 null/undefined 值） */
    sparse?: boolean;
}

/**
 * 主键索引配置选项
 */
export interface PrimaryIndexOptions {
    /** 索引名称 */
    name?: string;
    /** 主键字段名 */
    fieldName: string;
}

/**
 * 二级索引配置选项
 */
export interface SecondaryIndexOptions extends IndexOptions {
    /** 索引字段名 */
    fieldName: string;
    /** 是否唯一索引 */
    unique?: boolean;
}

/**
 * 索引统计信息
 */
export interface IndexStats {
    /** 索引名称 */
    name: string;
    /** 索引类型 */
    type: IndexType;
    /** 字段名 */
    fieldName: string;
    /** 键数量 */
    keyCount: number;
    /** 条目数量（二级索引可能有多个主键对应一个索引值） */
    entryCount: number;
    /** 是否唯一索引 */
    unique: boolean;
    /** 内存占用估算（字节） */
    memoryUsage: number;
}

/**
 * 索引查询结果
 */
export interface IndexQueryResult {
    /** 匹配的主键集合 */
    keys: Set<IndexKey>;
    /** 是否使用索引 */
    usedIndex: boolean;
    /** 使用的索引名称 */
    indexName?: string;
    /** 扫描的条目数 */
    scannedCount: number;
}

/**
 * B+ 树节点类型
 */
export enum BPlusTreeNodeType {
    /** 内部节点 */
    Internal = "internal",
    /** 叶子节点 */
    Leaf = "leaf",
}

/**
 * B+ 树节点接口
 */
export interface BPlusTreeNode<K = IndexKey> {
    /** 节点类型 */
    type: BPlusTreeNodeType;
    /** 键数组 */
    keys: K[];
    /** 是否为叶子节点 */
    isLeaf(): boolean;
}

/**
 * B+ 树叶子节点
 */
export interface BPlusTreeLeafNode<K = IndexKey, V = unknown> extends BPlusTreeNode<K> {
    /** 节点类型 */
    type: BPlusTreeNodeType.Leaf;
    /** 值数组 */
    values: V[];
    /** 下一个叶子节点（用于范围查询） */
    next?: BPlusTreeLeafNode<K, V>;
}

/**
 * B+ 树内部节点
 */
export interface BPlusTreeInternalNode<K = IndexKey, V = unknown> extends BPlusTreeNode<K> {
    /** 节点类型 */
    type: BPlusTreeNodeType.Internal;
    /** 子节点数组 */
    children: Array<BPlusTreeNode<K> | BPlusTreeLeafNode<K, V> | BPlusTreeInternalNode<K, V>>;
}

/**
 * 索引操作结果
 */
export interface IndexOperationResult {
    /** 操作是否成功 */
    success: boolean;
    /** 错误信息 */
    error?: string;
}

/**
 * 索引批量操作结果
 */
export interface IndexBatchResult {
    /** 操作是否成功 */
    success: boolean;
    /** 成功数量 */
    successCount: number;
    /** 失败数量 */
    failCount: number;
    /** 错误列表 */
    errors?: Array<{ key: IndexKey; error: string }>;
}

/**
 * 索引迭代器选项
 */
export interface IndexIteratorOptions {
    /** 是否逆序 */
    reverse?: boolean;
    /** 起始键 */
    start?: IndexKey;
    /** 结束键 */
    end?: IndexKey;
    /** 是否包含起始键 */
    includeStart?: boolean;
    /** 是否包含结束键 */
    includeEnd?: boolean;
}

/**
 * 键值对条目
 */
export interface IndexEntry<K = IndexKey, V = unknown> {
    /** 键 */
    key: K;
    /** 值 */
    value: V;
}

/**
 * 索引变更记录
 */
export interface IndexChangeRecord {
    /** 操作类型 */
    operation: "insert" | "update" | "delete";
    /** 主键 */
    primaryKey: IndexKey;
    /** 旧值（更新和删除时） */
    oldValue?: unknown;
    /** 新值（插入和更新时） */
    newValue?: unknown;
    /** 受影响的索引字段 */
    affectedFields: string[];
}

/**
 * 索引管理器配置
 */
export interface IndexManagerOptions {
    /** B+ 树阶数（默认 64） */
    bPlusTreeOrder?: number;
    /** 是否自动维护索引 */
    autoMaintain?: boolean;
}

/**
 * 比较器函数类型
 */
export type Comparator<K = IndexKey> = (a: K, b: K) => number;

/**
 * 键提取器函数类型
 */
export type KeyExtractor<T = unknown> = (record: T) => IndexKey;

/**
 * 值提取器函数类型
 */
export type ValueExtractor<T = unknown> = (record: T) => IndexValue;
