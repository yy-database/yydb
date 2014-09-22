/**
 * LightDB 主键索引实现
 * 基于 B+ 树的主键索引，提供高效的键值存储和范围查询
 */

import { BPlusTree } from "./BPlusTree";
import type { IndexKey, IndexStats, IndexIteratorOptions, Comparator } from "./types";
import { IndexType } from "./types";
import { KeyNotFoundError, DuplicateKeyError } from "./errors";

/**
 * 主键索引配置
 */
export interface PrimaryIndexConfig {
    /** 索引名称 */
    name: string;
    /** 字段名 */
    fieldName: string;
    /** B+ 树阶数 */
    order?: number;
    /** 自定义比较器 */
    comparator?: Comparator<IndexKey>;
}

/**
 * 主键索引
 * 使用 B+ 树实现，支持 O(log n) 的插入、查找、删除和范围查询
 */
export class PrimaryIndex<T = unknown> {
    private readonly tree: BPlusTree<IndexKey, T>;
    private readonly config: Required<Omit<PrimaryIndexConfig, "comparator">> & {
        comparator: Comparator<IndexKey> | undefined;
    };

    /**
     * 创建主键索引实例
     * @param config 索引配置
     */
    constructor(config: PrimaryIndexConfig) {
        this.config = {
            name: config.name,
            fieldName: config.fieldName,
            order: config.order ?? 64,
            comparator: config.comparator,
        };

        this.tree = new BPlusTree<IndexKey, T>(this.config.order, this.config.comparator);
    }

    /**
     * 获取索引名称
     */
    public get Name(): string {
        return this.config.name;
    }

    /**
     * 获取字段名
     */
    public get FieldName(): string {
        return this.config.fieldName;
    }

    /**
     * 获取索引类型
     */
    public get Type(): IndexType.Primary {
        return IndexType.Primary;
    }

    /**
     * 获取索引中的记录数量
     */
    public get Size(): number {
        return this.tree.Size;
    }

    /**
     * 检查索引是否为空
     */
    public isEmpty(): boolean {
        return this.tree.isEmpty();
    }

    /**
     * 插入键值对
     * @param key 主键
     * @param value 值
     * @param throwOnDuplicate 如果键已存在是否抛出异常
     * @returns 如果键已存在且不抛出异常，返回 false；否则返回 true
     */
    public insert(key: IndexKey, value: T, throwOnDuplicate: boolean = false): boolean {
        const existing = this.tree.find(key);
        if (existing !== undefined) {
            if (throwOnDuplicate) {
                throw new DuplicateKeyError(this.config.name, key);
            }
            return false;
        }

        this.tree.insert(key, value);
        return true;
    }

    /**
     * 更新键对应的值
     * @param key 主键
     * @param value 新值
     * @param throwOnMissing 如果键不存在是否抛出异常
     * @returns 如果键不存在且不抛出异常，返回 false；否则返回 true
     */
    public update(key: IndexKey, value: T, throwOnMissing: boolean = false): boolean {
        if (!this.tree.has(key)) {
            if (throwOnMissing) {
                throw new KeyNotFoundError(this.config.name, key);
            }
            return false;
        }

        return this.tree.update(key, value);
    }

    /**
     * 插入或更新键值对
     * @param key 主键
     * @param value 值
     */
    public upsert(key: IndexKey, value: T): void {
        if (this.tree.has(key)) {
            this.tree.update(key, value);
        } else {
            this.tree.insert(key, value);
        }
    }

    /**
     * 查找键对应的值
     * @param key 主键
     * @returns 值，如果不存在返回 undefined
     */
    public find(key: IndexKey): T | undefined {
        return this.tree.find(key);
    }

    /**
     * 检查键是否存在
     * @param key 主键
     */
    public has(key: IndexKey): boolean {
        return this.tree.has(key);
    }

    /**
     * 删除键值对
     * @param key 主键
     * @param throwOnMissing 如果键不存在是否抛出异常
     * @returns 被删除的值，如果不存在返回 undefined
     */
    public delete(key: IndexKey, throwOnMissing: boolean = false): T | undefined {
        const value = this.tree.delete(key);
        if (value === undefined && throwOnMissing) {
            throw new KeyNotFoundError(this.config.name, key);
        }
        return value;
    }

    /**
     * 范围查询
     * @param start 起始键
     * @param end 结束键
     * @param options 迭代选项
     * @returns 键值对数组
     */
    public range(start?: IndexKey, end?: IndexKey, options?: IndexIteratorOptions): Array<{ key: IndexKey; value: T }> {
        const includeStart = options?.includeStart ?? true;
        const includeEnd = options?.includeEnd ?? true;

        let result = this.tree.range(start, end, includeStart, includeEnd);

        if (options?.reverse) {
            result = result.reverse();
        }

        return result;
    }

    /**
     * 大于指定键的记录
     * @param key 键
     * @param include 是否包含该键
     */
    public greaterThan(key: IndexKey, include: boolean = false): Array<{ key: IndexKey; value: T }> {
        return this.tree.range(key, undefined, include, true);
    }

    /**
     * 小于指定键的记录
     * @param key 键
     * @param include 是否包含该键
     */
    public lessThan(key: IndexKey, include: boolean = false): Array<{ key: IndexKey; value: T }> {
        const result = this.tree.range(undefined, key, true, include);
        return result;
    }

    /**
     * 获取所有键（有序）
     */
    public keys(): IndexKey[] {
        return this.tree.keys();
    }

    /**
     * 获取所有值（按键顺序）
     */
    public values(): T[] {
        return this.tree.values();
    }

    /**
     * 获取所有键值对（有序）
     */
    public entries(): Array<{ key: IndexKey; value: T }> {
        return this.tree.entries();
    }

    /**
     * 获取最小键
     */
    public minKey(): IndexKey | undefined {
        return this.tree.minKey();
    }

    /**
     * 获取最大键
     */
    public maxKey(): IndexKey | undefined {
        return this.tree.maxKey();
    }

    /**
     * 获取第一个键值对
     */
    public first(): { key: IndexKey; value: T } | undefined {
        return this.tree.first();
    }

    /**
     * 获取最后一个键值对
     */
    public last(): { key: IndexKey; value: T } | undefined {
        return this.tree.last();
    }

    /**
     * 清空索引
     */
    public clear(): void {
        this.tree.clear();
    }

    /**
     * 遍历索引
     * @param callback 回调函数
     */
    public forEach(callback: (key: IndexKey, value: T) => void): void {
        this.tree.forEach(callback);
    }

    /**
     * 创建迭代器
     */
    public *[Symbol.iterator](): Generator<{ key: IndexKey; value: T }> {
        yield* this.tree;
    }

    /**
     * 获取索引统计信息
     */
    public getStats(): IndexStats {
        const keyCount = this.tree.Size;
        const memoryUsage = this.estimateMemoryUsage();

        return {
            name: this.config.name,
            type: IndexType.Primary,
            fieldName: this.config.fieldName,
            keyCount,
            entryCount: keyCount,
            unique: true,
            memoryUsage,
        };
    }

    /**
     * 批量插入
     * @param entries 键值对数组
     * @param throwOnDuplicate 如果键已存在是否抛出异常
     */
    public insertBatch(
        entries: Array<{ key: IndexKey; value: T }>,
        throwOnDuplicate: boolean = false,
    ): {
        success: number;
        failed: number;
        errors: Array<{ key: IndexKey; error: string }>;
    } {
        let success = 0;
        let failed = 0;
        const errors: Array<{ key: IndexKey; error: string }> = [];

        for (const { key, value } of entries) {
            try {
                if (this.insert(key, value, throwOnDuplicate)) {
                    success++;
                } else {
                    failed++;
                    errors.push({ key, error: "Duplicate key" });
                }
            } catch (error) {
                failed++;
                errors.push({
                    key,
                    error: error instanceof Error ? error.message : "Unknown error",
                });
            }
        }

        return { success, failed, errors };
    }

    /**
     * 批量删除
     * @param keys 键数组
     * @param throwOnMissing 如果键不存在是否抛出异常
     */
    public deleteBatch(
        keys: IndexKey[],
        throwOnMissing: boolean = false,
    ): {
        success: number;
        failed: number;
        errors: Array<{ key: IndexKey; error: string }>;
    } {
        let success = 0;
        let failed = 0;
        const errors: Array<{ key: IndexKey; error: string }> = [];

        for (const key of keys) {
            try {
                if (this.delete(key, throwOnMissing) !== undefined) {
                    success++;
                } else {
                    failed++;
                    errors.push({ key, error: "Key not found" });
                }
            } catch (error) {
                failed++;
                errors.push({
                    key,
                    error: error instanceof Error ? error.message : "Unknown error",
                });
            }
        }

        return { success, failed, errors };
    }

    /**
     * 估算内存使用
     */
    private estimateMemoryUsage(): number {
        const keyCount = this.tree.Size;
        const avgKeySize = 16;
        const avgValueSize = 64;
        const nodeOverhead = 128;
        const estimatedNodes = Math.ceil(keyCount / (this.config.order * 0.7));

        return keyCount * (avgKeySize + avgValueSize) + estimatedNodes * nodeOverhead;
    }

    /**
     * 验证索引一致性
     * 用于调试和测试
     */
    public validate(): { valid: boolean; errors: string[] } {
        const errors: string[] = [];
        const seenKeys = new Set<IndexKey>();

        let prevKey: IndexKey | undefined;
        let current = this.tree.first();

        while (current !== undefined) {
            if (seenKeys.has(current.key)) {
                errors.push(`Duplicate key found: ${current.key}`);
            }
            seenKeys.add(current.key);

            if (prevKey !== undefined) {
                const cmp = this.config.comparator ? this.config.comparator(prevKey, current.key) : defaultCompare(prevKey, current.key);

                if (cmp >= 0) {
                    errors.push(`Keys not in order: ${prevKey} >= ${current.key}`);
                }
            }

            prevKey = current.key;
            current = this.tree.entries().find((e) => e.key > current!.key);
        }

        if (seenKeys.size !== this.tree.Size) {
            errors.push(`Key count mismatch: expected ${this.tree.Size}, found ${seenKeys.size}`);
        }

        return {
            valid: errors.length === 0,
            errors,
        };
    }
}

/**
 * 默认比较函数
 */
function defaultCompare(a: IndexKey, b: IndexKey): number {
    if (typeof a === "number" && typeof b === "number") {
        return a - b;
    }
    const strA = String(a);
    const strB = String(b);
    if (strA < strB) return -1;
    if (strA > strB) return 1;
    return 0;
}
