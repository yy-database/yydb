/**
 * LightDB 二级索引实现
 * 使用 Map<索引值, Set<主键>> 结构，支持高效的等值查询
 */

import type { IndexKey, IndexValue, IndexStats, EqualityCondition } from "./types";
import { IndexType } from "./types";
import { DuplicateKeyError } from "./errors";

/**
 * 二级索引配置
 */
export interface SecondaryIndexConfig {
    /** 索引名称 */
    name: string;
    /** 字段名 */
    fieldName: string;
    /** 是否唯一索引 */
    unique?: boolean;
    /** 是否稀疏索引（不索引 null/undefined 值） */
    sparse?: boolean;
}

/**
 * 二级索引
 * 使用 Map + Set 结构实现，支持等值查询和范围查询
 */
export class SecondaryIndex {
    private readonly index: Map<string, Set<IndexKey>>;
    private readonly config: Required<SecondaryIndexConfig>;
    private keyCount: number = 0;

    /**
     * 创建二级索引实例
     * @param config 索引配置
     */
    constructor(config: SecondaryIndexConfig) {
        this.config = {
            name: config.name,
            fieldName: config.fieldName,
            unique: config.unique ?? false,
            sparse: config.sparse ?? false,
        };

        this.index = new Map();
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
    public get Type(): IndexType.Secondary | IndexType.Unique {
        return this.config.unique ? IndexType.Unique : IndexType.Secondary;
    }

    /**
     * 获取是否唯一索引
     */
    public get IsUnique(): boolean {
        return this.config.unique;
    }

    /**
     * 获取是否稀疏索引
     */
    public get IsSparse(): boolean {
        return this.config.sparse;
    }

    /**
     * 获取索引中的键数量
     */
    public get Size(): number {
        return this.keyCount;
    }

    /**
     * 获取索引值数量
     */
    public get ValueCount(): number {
        return this.index.size;
    }

    /**
     * 检查索引是否为空
     */
    public isEmpty(): boolean {
        return this.keyCount === 0;
    }

    /**
     * 插入索引条目
     * @param value 索引值
     * @param primaryKey 主键
     * @param throwOnDuplicate 如果是唯一索引且值已存在是否抛出异常
     * @returns 是否插入成功
     */
    public insert(value: IndexValue, primaryKey: IndexKey, throwOnDuplicate: boolean = false): boolean {
        if (value === null || value === undefined) {
            if (this.config.sparse) {
                return true;
            }
        }

        const valueKey = this.valueToKey(value);

        if (this.config.unique) {
            const existing = this.index.get(valueKey);
            if (existing && existing.size > 0) {
                if (throwOnDuplicate) {
                    throw new DuplicateKeyError(this.config.name, value);
                }
                return false;
            }
        }

        let keySet = this.index.get(valueKey);
        if (!keySet) {
            keySet = new Set<IndexKey>();
            this.index.set(valueKey, keySet);
        }

        if (keySet.has(primaryKey)) {
            return false;
        }

        keySet.add(primaryKey);
        this.keyCount++;
        return true;
    }

    /**
     * 删除索引条目
     * @param value 索引值
     * @param primaryKey 主键
     * @returns 是否删除成功
     */
    public delete(value: IndexValue, primaryKey: IndexKey): boolean {
        if (value === null || value === undefined) {
            if (this.config.sparse) {
                return true;
            }
        }

        const valueKey = this.valueToKey(value);
        const keySet = this.index.get(valueKey);

        if (!keySet) {
            return false;
        }

        const deleted = keySet.delete(primaryKey);
        if (deleted) {
            this.keyCount--;
        }

        if (keySet.size === 0) {
            this.index.delete(valueKey);
        }

        return deleted;
    }

    /**
     * 更新索引条目
     * @param oldValue 旧索引值
     * @param newValue 新索引值
     * @param primaryKey 主键
     * @param throwOnDuplicate 如果是唯一索引且新值已存在是否抛出异常
     * @returns 是否更新成功
     */
    public update(oldValue: IndexValue, newValue: IndexValue, primaryKey: IndexKey, throwOnDuplicate: boolean = false): boolean {
        const oldKey = this.valueToKey(oldValue);
        const newKey = this.valueToKey(newValue);

        if (oldKey === newKey) {
            return true;
        }

        if (this.config.unique) {
            const existing = this.index.get(newKey);
            if (existing && existing.size > 0) {
                if (throwOnDuplicate) {
                    throw new DuplicateKeyError(this.config.name, newValue);
                }
                return false;
            }
        }

        this.delete(oldValue, primaryKey);
        this.insert(newValue, primaryKey, throwOnDuplicate);
        return true;
    }

    /**
     * 等值查询
     * @param value 索引值
     * @returns 匹配的主键集合
     */
    public find(value: IndexValue): Set<IndexKey> {
        const valueKey = this.valueToKey(value);
        const keySet = this.index.get(valueKey);
        return keySet ? new Set(keySet) : new Set();
    }

    /**
     * 检查索引值是否存在
     * @param value 索引值
     */
    public has(value: IndexValue): boolean {
        const valueKey = this.valueToKey(value);
        const keySet = this.index.get(valueKey);
        return keySet !== undefined && keySet.size > 0;
    }

    /**
     * 检查索引值和主键组合是否存在
     * @param value 索引值
     * @param primaryKey 主键
     */
    public hasEntry(value: IndexValue, primaryKey: IndexKey): boolean {
        const valueKey = this.valueToKey(value);
        const keySet = this.index.get(valueKey);
        return keySet !== undefined && keySet.has(primaryKey);
    }

    /**
     * 条件查询
     * @param condition 查询条件
     * @returns 匹配的主键集合
     */
    public query(condition: EqualityCondition<IndexValue>): Set<IndexKey> {
        if (condition.$eq !== undefined) {
            return this.find(condition.$eq);
        }

        const result = new Set<IndexKey>();

        if (condition.$in !== undefined) {
            for (const value of condition.$in) {
                const keys = this.find(value);
                for (const key of keys) {
                    result.add(key);
                }
            }
        }

        if (condition.$ne !== undefined) {
            const excludeKeys = this.find(condition.$ne);
            for (const [_, keySet] of this.index) {
                for (const key of keySet) {
                    if (!excludeKeys.has(key)) {
                        result.add(key);
                    }
                }
            }
        }

        if (condition.$nin !== undefined) {
            const excludeKeys = new Set<IndexKey>();
            for (const value of condition.$nin) {
                const keys = this.find(value);
                for (const key of keys) {
                    excludeKeys.add(key);
                }
            }
            for (const [_, keySet] of this.index) {
                for (const key of keySet) {
                    if (!excludeKeys.has(key)) {
                        result.add(key);
                    }
                }
            }
        }

        return result;
    }

    /**
     * 范围查询
     * @param start 起始值
     * @param end 结束值
     * @param includeStart 是否包含起始值
     * @param includeEnd 是否包含结束值
     * @returns 匹配的主键集合
     */
    public range(start?: IndexValue, end?: IndexValue, includeStart: boolean = true, includeEnd: boolean = true): Set<IndexKey> {
        const result = new Set<IndexKey>();
        const sortedEntries = this.getSortedEntries();

        for (const { value, keys } of sortedEntries) {
            if (start !== undefined) {
                const cmp = this.compareValues(value, start);
                if (cmp < 0 || (cmp === 0 && !includeStart)) {
                    continue;
                }
            }

            if (end !== undefined) {
                const cmp = this.compareValues(value, end);
                if (cmp > 0 || (cmp === 0 && !includeEnd)) {
                    break;
                }
            }

            for (const key of keys) {
                result.add(key);
            }
        }

        return result;
    }

    /**
     * 获取所有索引值
     */
    public values(): IndexValue[] {
        const result: IndexValue[] = [];
        for (const key of this.index.keys()) {
            result.push(this.keyToValue(key));
        }
        return result;
    }

    /**
     * 获取所有主键
     */
    public keys(): IndexKey[] {
        const result: IndexKey[] = [];
        for (const keySet of this.index.values()) {
            for (const key of keySet) {
                result.push(key);
            }
        }
        return result;
    }

    /**
     * 获取所有条目
     */
    public entries(): Array<{ value: IndexValue; keys: Set<IndexKey> }> {
        const result: Array<{ value: IndexValue; keys: Set<IndexKey> }> = [];
        for (const [key, keySet] of this.index) {
            result.push({
                value: this.keyToValue(key),
                keys: new Set(keySet),
            });
        }
        return result;
    }

    /**
     * 清空索引
     */
    public clear(): void {
        this.index.clear();
        this.keyCount = 0;
    }

    /**
     * 获取索引统计信息
     */
    public getStats(): IndexStats {
        const valueCount = this.index.size;
        const memoryUsage = this.estimateMemoryUsage();

        return {
            name: this.config.name,
            type: this.Type,
            fieldName: this.config.fieldName,
            keyCount: this.keyCount,
            entryCount: valueCount,
            unique: this.config.unique,
            memoryUsage,
        };
    }

    /**
     * 批量插入
     * @param entries 条目数组
     * @param throwOnDuplicate 如果是唯一索引且值已存在是否抛出异常
     */
    public insertBatch(
        entries: Array<{ value: IndexValue; primaryKey: IndexKey }>,
        throwOnDuplicate: boolean = false,
    ): {
        success: number;
        failed: number;
        errors: Array<{ primaryKey: IndexKey; error: string }>;
    } {
        let success = 0;
        let failed = 0;
        const errors: Array<{ primaryKey: IndexKey; error: string }> = [];

        for (const { value, primaryKey } of entries) {
            try {
                if (this.insert(value, primaryKey, throwOnDuplicate)) {
                    success++;
                } else {
                    failed++;
                    errors.push({ primaryKey, error: "Duplicate or existing entry" });
                }
            } catch (error) {
                failed++;
                errors.push({
                    primaryKey,
                    error: error instanceof Error ? error.message : "Unknown error",
                });
            }
        }

        return { success, failed, errors };
    }

    /**
     * 批量删除
     * @param entries 条目数组
     */
    public deleteBatch(entries: Array<{ value: IndexValue; primaryKey: IndexKey }>): { success: number; failed: number } {
        let success = 0;
        let failed = 0;

        for (const { value, primaryKey } of entries) {
            if (this.delete(value, primaryKey)) {
                success++;
            } else {
                failed++;
            }
        }

        return { success, failed };
    }

    /**
     * 将索引值转换为存储键
     */
    private valueToKey(value: IndexValue): string {
        if (value === null) {
            return "\x00null";
        }
        if (value === undefined) {
            return "\x00undefined";
        }
        if (typeof value === "number") {
            return `\x01num:${value}`;
        }
        if (typeof value === "boolean") {
            return `\x02bool:${value}`;
        }
        return `\x03str:${value}`;
    }

    /**
     * 将存储键转换为索引值
     */
    private keyToValue(key: string): IndexValue {
        if (key === "\x00null") {
            return null;
        }
        if (key === "\x00undefined") {
            return null;
        }
        if (key.startsWith("\x01num:")) {
            return parseFloat(key.slice(6));
        }
        if (key.startsWith("\x02bool:")) {
            return key.slice(7) === "true";
        }
        if (key.startsWith("\x03str:")) {
            return key.slice(6);
        }
        return key;
    }

    /**
     * 比较两个索引值
     */
    private compareValues(a: IndexValue, b: IndexValue): number {
        if (a === null && b === null) return 0;
        if (a === null) return -1;
        if (b === null) return 1;

        if (a === undefined && b === undefined) return 0;
        if (a === undefined) return -1;
        if (b === undefined) return 1;

        const typeA = typeof a;
        const typeB = typeof b;

        if (typeA !== typeB) {
            return typeA.localeCompare(typeB);
        }

        if (typeA === "number") {
            return (a as number) - (b as number);
        }

        if (typeA === "boolean") {
            return ((a as boolean) ? 1 : 0) - ((b as boolean) ? 1 : 0);
        }

        const strA = String(a);
        const strB = String(b);
        if (strA < strB) return -1;
        if (strA > strB) return 1;
        return 0;
    }

    /**
     * 获取排序后的条目
     */
    private getSortedEntries(): Array<{
        value: IndexValue;
        keys: Set<IndexKey>;
    }> {
        const entries = this.entries();
        entries.sort((a, b) => this.compareValues(a.value, b.value));
        return entries;
    }

    /**
     * 估算内存使用
     */
    private estimateMemoryUsage(): number {
        const avgValueSize = 32;
        const avgKeySize = 16;
        const setOverhead = 64;
        const mapOverhead = 64;

        return this.index.size * (avgValueSize + mapOverhead) + this.keyCount * (avgKeySize + setOverhead);
    }

    /**
     * 验证索引一致性
     */
    public validate(): { valid: boolean; errors: string[] } {
        const errors: string[] = [];
        let countedKeys = 0;

        for (const [valueKey, keySet] of this.index) {
            if (keySet.size === 0) {
                errors.push(`Empty key set for value: ${valueKey}`);
            }

            if (this.config.unique && keySet.size > 1) {
                errors.push(`Unique index has multiple keys for value: ${valueKey}`);
            }

            countedKeys += keySet.size;
        }

        if (countedKeys !== this.keyCount) {
            errors.push(`Key count mismatch: expected ${this.keyCount}, found ${countedKeys}`);
        }

        return {
            valid: errors.length === 0,
            errors,
        };
    }
}
