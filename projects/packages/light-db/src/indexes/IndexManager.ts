/**
 * LightDB 索引管理器实现
 * 统一管理主键索引和二级索引，提供自动维护功能
 */

import { PrimaryIndex, PrimaryIndexConfig } from "./PrimaryIndex";
import { SecondaryIndex, SecondaryIndexConfig } from "./SecondaryIndex";
import type { IndexKey, IndexValue, IndexStats, IndexManagerOptions, EqualityCondition, KeyExtractor, ValueExtractor } from "./types";
import { IndexNotFoundError, IndexExistsError, IndexOperationError } from "./errors";

/**
 * 集合配置
 */
export interface CollectionIndexConfig<T = unknown> {
    /** 集合名称 */
    name: string;
    /** 主键字段名 */
    primaryKey: string;
    /** 主键提取器 */
    primaryKeyExtractor: KeyExtractor<T>;
    /** 二级索引配置 */
    secondaryIndexes?: Array<{
        name: string;
        fieldName: string;
        unique?: boolean;
        sparse?: boolean;
        valueExtractor: ValueExtractor<T>;
    }>;
}

/**
 * 索引管理器
 * 提供索引的创建、查询、维护等统一管理功能
 */
export class IndexManager<T = unknown> {
    private readonly options: Required<IndexManagerOptions>;
    private primaryIndex: PrimaryIndex<T> | null = null;
    private readonly secondaryIndexes: Map<string, SecondaryIndex>;
    private readonly valueExtractors: Map<string, ValueExtractor<T>>;
    private readonly collectionName: string;
    private readonly primaryKeyField: string;
    private readonly primaryKeyExtractor: KeyExtractor<T>;

    /**
     * 创建索引管理器实例
     * @param config 集合配置
     * @param options 索引管理器选项
     */
    constructor(config: CollectionIndexConfig<T>, options?: IndexManagerOptions) {
        this.options = {
            bPlusTreeOrder: options?.bPlusTreeOrder ?? 64,
            autoMaintain: options?.autoMaintain ?? true,
        };

        this.collectionName = config.name;
        this.primaryKeyField = config.primaryKey;
        this.primaryKeyExtractor = config.primaryKeyExtractor;
        this.secondaryIndexes = new Map();
        this.valueExtractors = new Map();

        this.initializePrimaryIndex();
        this.initializeSecondaryIndexes(config.secondaryIndexes ?? []);
    }

    /**
     * 获取集合名称
     */
    public get CollectionName(): string {
        return this.collectionName;
    }

    /**
     * 获取主键字段名
     */
    public get PrimaryKeyField(): string {
        return this.primaryKeyField;
    }

    /**
     * 获取主键索引
     */
    public get PrimaryIndex(): PrimaryIndex<T> {
        if (!this.primaryIndex) {
            throw new IndexOperationError("Primary index not initialized", "getPrimaryIndex");
        }
        return this.primaryIndex;
    }

    /**
     * 获取记录数量
     */
    public get Size(): number {
        return this.primaryIndex?.Size ?? 0;
    }

    /**
     * 初始化主键索引
     */
    private initializePrimaryIndex(): void {
        const config: PrimaryIndexConfig = {
            name: `${this.collectionName}_pk`,
            fieldName: this.primaryKeyField,
            order: this.options.bPlusTreeOrder,
        };

        this.primaryIndex = new PrimaryIndex<T>(config);
    }

    /**
     * 初始化二级索引
     */
    private initializeSecondaryIndexes(
        configs: Array<{
            name: string;
            fieldName: string;
            unique?: boolean;
            sparse?: boolean;
            valueExtractor: ValueExtractor<T>;
        }>,
    ): void {
        for (const config of configs) {
            this.createSecondaryIndex(config);
        }
    }

    /**
     * 创建二级索引
     * @param config 索引配置
     */
    public createSecondaryIndex(config: {
        name: string;
        fieldName: string;
        unique?: boolean;
        sparse?: boolean;
        valueExtractor: ValueExtractor<T>;
    }): SecondaryIndex {
        if (this.secondaryIndexes.has(config.name)) {
            throw new IndexExistsError(config.name);
        }

        const indexConfig: SecondaryIndexConfig = {
            name: config.name,
            fieldName: config.fieldName,
            unique: config.unique ?? false,
            sparse: config.sparse ?? false,
        };

        const index = new SecondaryIndex(indexConfig);
        this.secondaryIndexes.set(config.name, index);
        this.valueExtractors.set(config.name, config.valueExtractor);

        return index;
    }

    /**
     * 删除二级索引
     * @param name 索引名称
     */
    public dropSecondaryIndex(name: string): boolean {
        const index = this.secondaryIndexes.get(name);
        if (!index) {
            return false;
        }

        index.clear();
        this.secondaryIndexes.delete(name);
        this.valueExtractors.delete(name);
        return true;
    }

    /**
     * 获取二级索引
     * @param name 索引名称
     */
    public getSecondaryIndex(name: string): SecondaryIndex {
        const index = this.secondaryIndexes.get(name);
        if (!index) {
            throw new IndexNotFoundError(name);
        }
        return index;
    }

    /**
     * 检查二级索引是否存在
     * @param name 索引名称
     */
    public hasSecondaryIndex(name: string): boolean {
        return this.secondaryIndexes.has(name);
    }

    /**
     * 获取所有二级索引名称
     */
    public getSecondaryIndexNames(): string[] {
        return Array.from(this.secondaryIndexes.keys());
    }

    /**
     * 插入记录
     * @param record 记录
     * @param throwOnDuplicate 如果主键已存在是否抛出异常
     * @returns 是否插入成功
     */
    public insert(record: T, throwOnDuplicate: boolean = false): boolean {
        const primaryKey = this.primaryKeyExtractor(record);

        if (!this.primaryIndex!.insert(primaryKey, record, throwOnDuplicate)) {
            return false;
        }

        for (const [indexName, index] of this.secondaryIndexes) {
            const extractor = this.valueExtractors.get(indexName);
            if (extractor) {
                const value = extractor(record);
                try {
                    index.insert(value, primaryKey, index.IsUnique && throwOnDuplicate);
                } catch (error) {
                    this.primaryIndex!.delete(primaryKey);
                    throw error;
                }
            }
        }

        return true;
    }

    /**
     * 更新记录
     * @param oldRecord 旧记录
     * @param newRecord 新记录
     * @param throwOnMissing 如果主键不存在是否抛出异常
     * @returns 是否更新成功
     */
    public update(oldRecord: T, newRecord: T, throwOnMissing: boolean = false): boolean {
        const oldPrimaryKey = this.primaryKeyExtractor(oldRecord);
        const newPrimaryKey = this.primaryKeyExtractor(newRecord);

        if (!this.primaryIndex!.has(oldPrimaryKey)) {
            if (throwOnMissing) {
                throw new IndexOperationError(`Record not found with primary key: ${oldPrimaryKey}`, "update");
            }
            return false;
        }

        if (oldPrimaryKey !== newPrimaryKey) {
            this.primaryIndex!.delete(oldPrimaryKey);
            this.primaryIndex!.insert(newPrimaryKey, newRecord, true);

            for (const [indexName, index] of this.secondaryIndexes) {
                const extractor = this.valueExtractors.get(indexName);
                if (extractor) {
                    const oldValue = extractor(oldRecord);
                    const newValue = extractor(newRecord);
                    index.delete(oldValue, oldPrimaryKey);
                    index.insert(newValue, newPrimaryKey, index.IsUnique);
                }
            }
        } else {
            this.primaryIndex!.update(oldPrimaryKey, newRecord);

            for (const [indexName, index] of this.secondaryIndexes) {
                const extractor = this.valueExtractors.get(indexName);
                if (extractor) {
                    const oldValue = extractor(oldRecord);
                    const newValue = extractor(newRecord);
                    index.update(oldValue, newValue, oldPrimaryKey, index.IsUnique);
                }
            }
        }

        return true;
    }

    /**
     * 删除记录
     * @param record 要删除的记录
     * @param throwOnMissing 如果主键不存在是否抛出异常
     * @returns 被删除的记录，如果不存在返回 undefined
     */
    public delete(record: T, throwOnMissing: boolean = false): T | undefined {
        const primaryKey = this.primaryKeyExtractor(record);

        const deleted = this.primaryIndex!.delete(primaryKey, throwOnMissing);
        if (deleted === undefined) {
            return undefined;
        }

        for (const [indexName, index] of this.secondaryIndexes) {
            const extractor = this.valueExtractors.get(indexName);
            if (extractor) {
                const value = extractor(record);
                index.delete(value, primaryKey);
            }
        }

        return deleted;
    }

    /**
     * 通过主键删除记录
     * @param primaryKey 主键
     * @param throwOnMissing 如果主键不存在是否抛出异常
     * @returns 被删除的记录，如果不存在返回 undefined
     */
    public deleteByKey(primaryKey: IndexKey, throwOnMissing: boolean = false): T | undefined {
        const record = this.primaryIndex!.find(primaryKey);
        if (record === undefined) {
            if (throwOnMissing) {
                throw new IndexOperationError(`Record not found with primary key: ${primaryKey}`, "deleteByKey");
            }
            return undefined;
        }

        return this.delete(record, throwOnMissing);
    }

    /**
     * 通过主键查找记录
     * @param primaryKey 主键
     */
    public findByPrimaryKey(primaryKey: IndexKey): T | undefined {
        return this.primaryIndex!.find(primaryKey);
    }

    /**
     * 检查主键是否存在
     * @param primaryKey 主键
     */
    public hasByPrimaryKey(primaryKey: IndexKey): boolean {
        return this.primaryIndex!.has(primaryKey);
    }

    /**
     * 通过二级索引查找
     * @param indexName 索引名称
     * @param value 索引值
     * @returns 匹配的记录数组
     */
    public findBySecondaryIndex(indexName: string, value: IndexValue): T[] {
        const index = this.getSecondaryIndex(indexName);
        const primaryKeys = index.find(value);
        const results: T[] = [];

        for (const key of primaryKeys) {
            const record = this.primaryIndex!.find(key);
            if (record !== undefined) {
                results.push(record);
            }
        }

        return results;
    }

    /**
     * 通过二级索引条件查询
     * @param indexName 索引名称
     * @param condition 查询条件
     * @returns 匹配的记录数组
     */
    public queryBySecondaryIndex(indexName: string, condition: EqualityCondition<IndexValue>): T[] {
        const index = this.getSecondaryIndex(indexName);
        const primaryKeys = index.query(condition);
        const results: T[] = [];

        for (const key of primaryKeys) {
            const record = this.primaryIndex!.find(key);
            if (record !== undefined) {
                results.push(record);
            }
        }

        return results;
    }

    /**
     * 主键范围查询
     * @param start 起始键
     * @param end 结束键
     * @param includeStart 是否包含起始键
     * @param includeEnd 是否包含结束键
     * @returns 匹配的记录数组
     */
    public rangeByPrimaryKey(start?: IndexKey, end?: IndexKey, includeStart: boolean = true, includeEnd: boolean = true): T[] {
        const entries = this.primaryIndex!.range(start, end, {
            includeStart,
            includeEnd,
        });

        return entries.map((entry) => entry.value);
    }

    /**
     * 二级索引范围查询
     * @param indexName 索引名称
     * @param start 起始值
     * @param end 结束值
     * @param includeStart 是否包含起始值
     * @param includeEnd 是否包含结束值
     * @returns 匹配的记录数组
     */
    public rangeBySecondaryIndex(
        indexName: string,
        start?: IndexValue,
        end?: IndexValue,
        includeStart: boolean = true,
        includeEnd: boolean = true,
    ): T[] {
        const index = this.getSecondaryIndex(indexName);
        const primaryKeys = index.range(start, end, includeStart, includeEnd);
        const results: T[] = [];

        for (const key of primaryKeys) {
            const record = this.primaryIndex!.find(key);
            if (record !== undefined) {
                results.push(record);
            }
        }

        return results;
    }

    /**
     * 获取所有记录
     */
    public getAll(): T[] {
        return this.primaryIndex!.values();
    }

    /**
     * 获取所有主键
     */
    public getAllPrimaryKeys(): IndexKey[] {
        return this.primaryIndex!.keys();
    }

    /**
     * 清空所有索引
     */
    public clear(): void {
        this.primaryIndex!.clear();
        for (const index of this.secondaryIndexes.values()) {
            index.clear();
        }
    }

    /**
     * 获取所有索引统计信息
     */
    public getStats(): IndexStats[] {
        const stats: IndexStats[] = [];

        if (this.primaryIndex) {
            stats.push(this.primaryIndex.getStats());
        }

        for (const index of this.secondaryIndexes.values()) {
            stats.push(index.getStats());
        }

        return stats;
    }

    /**
     * 批量插入
     * @param records 记录数组
     * @param throwOnDuplicate 如果主键已存在是否抛出异常
     */
    public insertBatch(
        records: T[],
        throwOnDuplicate: boolean = false,
    ): {
        success: number;
        failed: number;
        errors: Array<{ index: number; error: string }>;
    } {
        let success = 0;
        let failed = 0;
        const errors: Array<{ index: number; error: string }> = [];

        for (let i = 0; i < records.length; i++) {
            try {
                if (this.insert(records[i]!, throwOnDuplicate)) {
                    success++;
                } else {
                    failed++;
                    errors.push({ index: i, error: "Duplicate key" });
                }
            } catch (error) {
                failed++;
                errors.push({
                    index: i,
                    error: error instanceof Error ? error.message : "Unknown error",
                });
            }
        }

        return { success, failed, errors };
    }

    /**
     * 批量删除
     * @param primaryKeys 主键数组
     */
    public deleteBatch(primaryKeys: IndexKey[]): {
        success: number;
        failed: number;
    } {
        let success = 0;
        let failed = 0;

        for (const key of primaryKeys) {
            if (this.deleteByKey(key) !== undefined) {
                success++;
            } else {
                failed++;
            }
        }

        return { success, failed };
    }

    /**
     * 验证索引一致性
     */
    public validate(): {
        valid: boolean;
        errors: Array<{ index: string; errors: string[] }>;
    } {
        const results: Array<{ index: string; errors: string[] }> = [];
        let allValid = true;

        if (this.primaryIndex) {
            const primaryValidation = this.primaryIndex.validate();
            if (!primaryValidation.valid) {
                allValid = false;
                results.push({
                    index: this.primaryIndex.Name,
                    errors: primaryValidation.errors,
                });
            }
        }

        for (const [name, index] of this.secondaryIndexes) {
            const validation = index.validate();
            if (!validation.valid) {
                allValid = false;
                results.push({
                    index: name,
                    errors: validation.errors,
                });
            }
        }

        for (const [indexName, index] of this.secondaryIndexes) {
            const orphanKeys: IndexKey[] = [];

            for (const { keys } of index.entries()) {
                for (const key of keys) {
                    if (!this.primaryIndex!.has(key)) {
                        orphanKeys.push(key);
                    }
                }
            }

            if (orphanKeys.length > 0) {
                allValid = false;
                results.push({
                    index: indexName,
                    errors: [`Orphan keys found: ${orphanKeys.slice(0, 10).join(", ")}${orphanKeys.length > 10 ? "..." : ""}`],
                });
            }
        }

        return {
            valid: allValid,
            errors: results,
        };
    }

    /**
     * 重建所有索引
     * @param records 所有记录
     */
    public rebuild(records: T[]): void {
        this.clear();

        for (const record of records) {
            const primaryKey = this.primaryKeyExtractor(record);
            this.primaryIndex!.insert(primaryKey, record);

            for (const [indexName, index] of this.secondaryIndexes) {
                const extractor = this.valueExtractors.get(indexName);
                if (extractor) {
                    const value = extractor(record);
                    index.insert(value, primaryKey);
                }
            }
        }
    }

    /**
     * 重建指定二级索引
     * @param indexName 索引名称
     */
    public rebuildSecondaryIndex(indexName: string): void {
        const index = this.getSecondaryIndex(indexName);
        const extractor = this.valueExtractors.get(indexName);

        if (!extractor) {
            throw new IndexOperationError(`Value extractor not found for index: ${indexName}`, "rebuildSecondaryIndex");
        }

        index.clear();

        for (const record of this.primaryIndex!.values()) {
            const primaryKey = this.primaryKeyExtractor(record);
            const value = extractor(record);
            index.insert(value, primaryKey);
        }
    }

    /**
     * 获取估算的内存使用量
     */
    public getMemoryUsage(): number {
        let total = 0;

        if (this.primaryIndex) {
            total += this.primaryIndex.getStats().memoryUsage;
        }

        for (const index of this.secondaryIndexes.values()) {
            total += index.getStats().memoryUsage;
        }

        return total;
    }
}
