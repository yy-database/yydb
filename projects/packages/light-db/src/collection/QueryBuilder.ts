/**
 * LightDB 查询构建器
 * 提供链式查询 API
 */

import type { QueryFilter, SortCondition, SortDirection, FindResult, PrimaryKeyType } from "./types";
import { QueryParser } from "./QueryParser";

/**
 * 查询执行器接口
 * 用于解耦 QueryBuilder 和 Collection
 */
export interface QueryExecutor<T extends object> {
    /** 获取所有文档 */
    getAll(): T[];
    /** 通过主键查找 */
    findByPrimaryKey(key: PrimaryKeyType): T | undefined;
    /** 通过二级索引查找 */
    findBySecondaryIndex(indexName: string, value: unknown): T[];
    /** 获取主键字段名 */
    getPrimaryKeyField(): string;
    /** 获取二级索引字段列表 */
    getSecondaryIndexFields(): string[];
    /** 提取主键值 */
    extractPrimaryKey(doc: T): PrimaryKeyType;
}

/**
 * 查询构建器
 * 提供链式查询 API
 */
export class QueryBuilder<T extends object> {
    private readonly executor: QueryExecutor<T>;
    private filter: QueryFilter<T>;
    private sortCondition: SortCondition<T>;
    private skipValue: number;
    private limitValue: number;
    private fieldsValue: (keyof T)[] | null;

    /**
     * 创建查询构建器实例
     * @param executor 查询执行器
     */
    constructor(executor: QueryExecutor<T>) {
        this.executor = executor;
        this.filter = {};
        this.sortCondition = {};
        this.skipValue = 0;
        this.limitValue = 0;
        this.fieldsValue = null;
    }

    /**
     * 设置查询条件
     * @param condition 查询条件
     */
    public where(condition: QueryFilter<T>): this {
        this.filter = { ...this.filter, ...condition };
        return this;
    }

    /**
     * 添加等于条件
     * @param field 字段名
     * @param value 值
     */
    public eq<K extends keyof T>(field: K, value: T[K]): this {
        const currentFilter = this.filter[field as keyof QueryFilter<T>];
        if (typeof currentFilter === "object" && currentFilter !== null) {
            (currentFilter as Record<string, unknown>)["$eq"] = value;
        } else {
            (this.filter as Record<string, unknown>)[field as string] = value;
        }
        return this;
    }

    /**
     * 添加不等于条件
     * @param field 字段名
     * @param value 值
     */
    public ne<K extends keyof T>(field: K, value: T[K]): this {
        const fieldCondition = this.getOrCreateFieldCondition(field);
        fieldCondition["$ne"] = value;
        return this;
    }

    /**
     * 添加大于条件
     * @param field 字段名
     * @param value 值
     */
    public gt<K extends keyof T>(field: K, value: T[K]): this {
        const fieldCondition = this.getOrCreateFieldCondition(field);
        fieldCondition["$gt"] = value;
        return this;
    }

    /**
     * 添加大于等于条件
     * @param field 字段名
     * @param value 值
     */
    public gte<K extends keyof T>(field: K, value: T[K]): this {
        const fieldCondition = this.getOrCreateFieldCondition(field);
        fieldCondition["$gte"] = value;
        return this;
    }

    /**
     * 添加小于条件
     * @param field 字段名
     * @param value 值
     */
    public lt<K extends keyof T>(field: K, value: T[K]): this {
        const fieldCondition = this.getOrCreateFieldCondition(field);
        fieldCondition["$lt"] = value;
        return this;
    }

    /**
     * 添加小于等于条件
     * @param field 字段名
     * @param value 值
     */
    public lte<K extends keyof T>(field: K, value: T[K]): this {
        const fieldCondition = this.getOrCreateFieldCondition(field);
        fieldCondition["$lte"] = value;
        return this;
    }

    /**
     * 添加包含于数组条件
     * @param field 字段名
     * @param values 值数组
     */
    public in<K extends keyof T>(field: K, values: T[K][]): this {
        const fieldCondition = this.getOrCreateFieldCondition(field);
        fieldCondition["$in"] = values;
        return this;
    }

    /**
     * 添加不包含于数组条件
     * @param field 字段名
     * @param values 值数组
     */
    public nin<K extends keyof T>(field: K, values: T[K][]): this {
        const fieldCondition = this.getOrCreateFieldCondition(field);
        fieldCondition["$nin"] = values;
        return this;
    }

    /**
     * 添加存在性检查条件
     * @param field 字段名
     * @param exists 是否存在
     */
    public fieldExists<K extends keyof T>(field: K, exists: boolean = true): this {
        const fieldCondition = this.getOrCreateFieldCondition(field);
        fieldCondition["$exists"] = exists;
        return this;
    }

    /**
     * 添加正则表达式匹配条件
     * @param field 字段名
     * @param pattern 正则表达式
     * @param options 正则表达式选项
     */
    public regex<K extends keyof T>(field: K, pattern: RegExp | string, options?: string): this {
        const fieldCondition = this.getOrCreateFieldCondition(field);
        fieldCondition["$regex"] = pattern;
        if (options) {
            fieldCondition["$options"] = options;
        }
        return this;
    }

    /**
     * 添加逻辑与条件
     * @param conditions 条件数组
     */
    public and(...conditions: QueryFilter<T>[]): this {
        if (!this.filter.$and) {
            this.filter.$and = [];
        }
        this.filter.$and.push(...conditions);
        return this;
    }

    /**
     * 添加逻辑或条件
     * @param conditions 条件数组
     */
    public or(...conditions: QueryFilter<T>[]): this {
        if (!this.filter.$or) {
            this.filter.$or = [];
        }
        this.filter.$or.push(...conditions);
        return this;
    }

    /**
     * 添加逻辑非条件
     * @param condition 条件
     */
    public not(condition: QueryFilter<T>): this {
        this.filter.$not = condition;
        return this;
    }

    /**
     * 设置排序条件
     * @param field 字段名
     * @param direction 排序方向
     */
    public sortBy<K extends keyof T>(field: K, direction: SortDirection = 1): this {
        this.sortCondition[field] = direction;
        return this;
    }

    /**
     * 升序排序
     * @param field 字段名
     */
    public asc<K extends keyof T>(field: K): this {
        return this.sortBy(field, "asc");
    }

    /**
     * 降序排序
     * @param field 字段名
     */
    public desc<K extends keyof T>(field: K): this {
        return this.sortBy(field, "desc");
    }

    /**
     * 设置跳过记录数
     * @param count 跳过的记录数
     */
    public skip(count: number): this {
        this.skipValue = Math.max(0, count);
        return this;
    }

    /**
     * 设置返回记录数限制
     * @param count 最大返回记录数
     */
    public limit(count: number): this {
        this.limitValue = Math.max(0, count);
        return this;
    }

    /**
     * 设置返回的字段
     * @param fields 字段列表
     */
    public select<K extends keyof T>(...fields: K[]): this {
        this.fieldsValue = fields;
        return this;
    }

    /**
     * 获取或创建字段条件对象
     */
    private getOrCreateFieldCondition<K extends keyof T>(field: K): Record<string, unknown> {
        const fieldKey = field as string;
        let condition = (this.filter as Record<string, unknown>)[fieldKey];

        if (!condition || typeof condition !== "object" || Array.isArray(condition)) {
            condition = {};
            (this.filter as Record<string, unknown>)[fieldKey] = condition;
        }

        return condition as Record<string, unknown>;
    }

    /**
     * 执行查询并返回所有匹配的文档
     */
    public toArray(): FindResult<T> {
        const result = this.execute();
        const output: FindResult<T> = {
            docs: result.docs,
        };

        if (result.total !== undefined) {
            output.total = result.total;
        }
        if (result.usedIndex !== undefined) {
            output.usedIndex = result.usedIndex;
        }
        if (result.indexName !== undefined) {
            output.indexName = result.indexName;
        }

        return output;
    }

    /**
     * 执行查询并返回第一条匹配的文档
     */
    public first(): T | null {
        this.limitValue = 1;
        const result = this.execute();
        return result.docs.length > 0 ? (result.docs[0] ?? null) : null;
    }

    /**
     * 执行查询并返回最后一条匹配的文档
     */
    public last(): T | null {
        this.limitValue = 1;
        const result = this.execute();
        return result.docs.length > 0 ? (result.docs[result.docs.length - 1] ?? null) : null;
    }

    /**
     * 统计匹配的文档数量
     */
    public count(): number {
        const result = this.execute();
        return result.total ?? result.docs.length;
    }

    /**
     * 检查是否存在匹配的文档
     */
    public hasMatch(): boolean {
        return this.count() > 0;
    }

    /**
     * 执行查询
     */
    private execute(): FindResult<T> {
        const indexFields = [this.executor.getPrimaryKeyField(), ...this.executor.getSecondaryIndexFields()];

        const analysis = QueryParser.analyzeQuery(this.filter, indexFields);

        let docs: T[];
        let usedIndex: boolean | undefined;
        let indexName: string | undefined;

        if (analysis.canUseIndex && analysis.indexField) {
            const result = this.executeWithIndex(analysis);
            docs = result.docs;
            usedIndex = result.usedIndex;
            indexName = result.indexName;
        } else {
            docs = this.executeWithScan();
        }

        const total = docs.length;

        if (Object.keys(this.sortCondition).length > 0) {
            docs = this.applySort(docs);
        }

        if (this.skipValue > 0) {
            docs = docs.slice(this.skipValue);
        }

        if (this.limitValue > 0) {
            docs = docs.slice(0, this.limitValue);
        }

        if (this.fieldsValue && this.fieldsValue.length > 0) {
            docs = this.applyProjection(docs);
        }

        const output: FindResult<T> = {
            docs,
            total,
        };

        if (usedIndex !== undefined) {
            output.usedIndex = usedIndex;
        }
        if (indexName !== undefined) {
            output.indexName = indexName;
        }

        return output;
    }

    /**
     * 使用索引执行查询
     */
    private executeWithIndex(analysis: {
        indexField?: string;
        indexValue?: PrimaryKeyType | PrimaryKeyType[];
        isExactMatch: boolean;
    }): { docs: T[]; usedIndex: boolean; indexName?: string } {
        const primaryKeyField = this.executor.getPrimaryKeyField();

        if (analysis.indexField === primaryKeyField) {
            if (analysis.isExactMatch && analysis.indexValue !== undefined) {
                const doc = this.executor.findByPrimaryKey(analysis.indexValue as PrimaryKeyType);
                if (doc && QueryParser.matches(doc, this.filter)) {
                    const output: { docs: T[]; usedIndex: boolean; indexName?: string } = {
                        docs: [doc],
                        usedIndex: true,
                    };
                    output.indexName = primaryKeyField;
                    return output;
                }
                const output: { docs: T[]; usedIndex: boolean; indexName?: string } = {
                    docs: [],
                    usedIndex: true,
                };
                output.indexName = primaryKeyField;
                return output;
            }
        }

        const secondaryIndexFields = this.executor.getSecondaryIndexFields();
        if (secondaryIndexFields.includes(analysis.indexField!)) {
            if (analysis.isExactMatch && analysis.indexValue !== undefined && analysis.indexField) {
                const docs = this.executor.findBySecondaryIndex(analysis.indexField, analysis.indexValue);
                const filtered = docs.filter((doc) => QueryParser.matches(doc, this.filter));
                const output: { docs: T[]; usedIndex: boolean; indexName?: string } = {
                    docs: filtered,
                    usedIndex: true,
                };
                output.indexName = analysis.indexField;
                return output;
            }
        }

        return { docs: this.executeWithScan(), usedIndex: false };
    }

    /**
     * 使用全表扫描执行查询
     */
    private executeWithScan(): T[] {
        const allDocs = this.executor.getAll();
        return allDocs.filter((doc) => QueryParser.matches(doc, this.filter));
    }

    /**
     * 应用排序
     */
    private applySort(docs: T[]): T[] {
        const sortEntries = Object.entries(this.sortCondition) as [keyof T, SortDirection][];

        if (sortEntries.length === 0) {
            return docs;
        }

        return docs.slice().sort((a, b) => {
            for (const [field, direction] of sortEntries) {
                const aVal = a[field];
                const bVal = b[field];
                const cmp = this.compareValues(aVal, bVal);

                if (cmp !== 0) {
                    const dir = direction === "desc" || direction === -1 ? -1 : 1;
                    return cmp * dir;
                }
            }
            return 0;
        });
    }

    /**
     * 比较两个值
     */
    private compareValues(a: unknown, b: unknown): number {
        if (a === b) return 0;
        if (a === null || a === undefined) return -1;
        if (b === null || b === undefined) return 1;

        if (typeof a === "number" && typeof b === "number") {
            return a - b;
        }

        if (typeof a === "string" && typeof b === "string") {
            return a.localeCompare(b);
        }

        if (a instanceof Date && b instanceof Date) {
            return a.getTime() - b.getTime();
        }

        if (typeof a === "boolean" && typeof b === "boolean") {
            return a === b ? 0 : a ? 1 : -1;
        }

        return 0;
    }

    /**
     * 应用投影
     */
    private applyProjection(docs: T[]): T[] {
        if (!this.fieldsValue || this.fieldsValue.length === 0) {
            return docs;
        }

        return docs.map((doc) => {
            const projected: Partial<T> = {};
            for (const field of this.fieldsValue!) {
                if (field in doc) {
                    projected[field] = doc[field];
                }
            }
            return projected as T;
        });
    }

    /**
     * 获取当前查询条件
     */
    public getFilter(): QueryFilter<T> {
        return { ...this.filter };
    }

    /**
     * 获取当前排序条件
     */
    public getSort(): SortCondition<T> {
        return { ...this.sortCondition };
    }

    /**
     * 获取当前跳过值
     */
    public getSkip(): number {
        return this.skipValue;
    }

    /**
     * 获取当前限制值
     */
    public getLimit(): number {
        return this.limitValue;
    }

    /**
     * 重置查询构建器
     */
    public reset(): this {
        this.filter = {};
        this.sortCondition = {};
        this.skipValue = 0;
        this.limitValue = 0;
        this.fieldsValue = null;
        return this;
    }

    /**
     * 克隆查询构建器
     */
    public clone(): QueryBuilder<T> {
        const cloned = new QueryBuilder<T>(this.executor);
        cloned.filter = { ...this.filter };
        cloned.sortCondition = { ...this.sortCondition };
        cloned.skipValue = this.skipValue;
        cloned.limitValue = this.limitValue;
        cloned.fieldsValue = this.fieldsValue ? [...this.fieldsValue] : null;
        return cloned;
    }
}
