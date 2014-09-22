/**
 * LightDB 集合类
 * 提供类型安全的 CRUD 操作和链式查询 API
 */

import { IndexManager, CollectionIndexConfig } from "../indexes";
import type { IndexKey, IndexValue } from "../indexes";
import { QueryBuilder, QueryExecutor } from "./QueryBuilder";
import { QueryParser } from "./QueryParser";
import type {
    CollectionOptions,
    InsertOptions,
    InsertResult,
    InsertManyResult,
    UpdateOptions,
    UpdateResult,
    DeleteOptions,
    DeleteResult,
    QueryFilter,
    QueryOptions,
    FindResult,
    UpdateOperation,
    UpdateOperators,
    PrimaryKeyType,
    CollectionStats,
    CollectionEventListener,
    CollectionEventType,
    IndexConfig,
} from "./types";

/**
 * 集合错误类
 */
export class CollectionError extends Error {
    public readonly code: string;

    constructor(message: string, code: string) {
        super(message);
        this.name = "CollectionError";
        this.code = code;
        Object.setPrototypeOf(this, CollectionError.prototype);
    }
}

/**
 * LightDB 集合
 * 类似数据库表的概念，提供类型安全的 CRUD 操作
 */
export class Collection<T extends Record<string, unknown>> implements QueryExecutor<T> {
    private readonly name: string;
    private readonly primaryKeyField: keyof T & string;
    private readonly indexManager: IndexManager<T>;
    private readonly eventListeners: Map<CollectionEventType, Set<CollectionEventListener<T>>>;

    /**
     * 创建集合实例
     * @param options 集合配置选项
     */
    constructor(options: CollectionOptions<T>) {
        this.name = options.name ?? "default";
        this.primaryKeyField = options.primaryKey;
        this.eventListeners = new Map();

        const indexConfig: CollectionIndexConfig<T> = {
            name: this.name,
            primaryKey: this.primaryKeyField,
            primaryKeyExtractor: (doc: T) => this.extractPrimaryKey(doc) as IndexKey,
            secondaryIndexes: (options.indexes ?? []).map((idx) => ({
                name: idx.name,
                fieldName: idx.fieldName,
                unique: idx.unique ?? false,
                sparse: idx.sparse ?? false,
                valueExtractor: (doc: T) => doc[idx.fieldName] as IndexValue,
            })),
        };

        this.indexManager = new IndexManager<T>(indexConfig);
    }

    /**
     * 获取集合名称
     */
    public get Name(): string {
        return this.name;
    }

    /**
     * 获取主键字段名
     */
    public get PrimaryKeyField(): string {
        return this.primaryKeyField;
    }

    /**
     * 获取文档数量
     */
    public get Size(): number {
        return this.indexManager.Size;
    }

    /**
     * 插入单条文档
     * @param doc 要插入的文档
     * @param options 插入选项
     */
    public insert(doc: T, options: InsertOptions = {}): InsertResult<T> {
        const primaryKey = this.extractPrimaryKey(doc);

        if (this.indexManager.hasByPrimaryKey(primaryKey)) {
            if (options.throwOnDuplicate) {
                throw new CollectionError(`Document with primary key "${primaryKey}" already exists`, "DUPLICATE_KEY");
            }
            return {
                success: false,
                insertedCount: 0,
                insertedDocs: [],
            };
        }

        this.indexManager.insert(doc, true);
        this.emitEvent("insert", { doc });

        return {
            success: true,
            insertedCount: 1,
            insertedDocs: [doc],
        };
    }

    /**
     * 批量插入文档
     * @param docs 要插入的文档数组
     * @param options 插入选项
     */
    public insertMany(docs: T[], options: InsertOptions = {}): InsertManyResult<T> {
        const insertedDocs: T[] = [];
        const errors: Array<{ index: number; error: string }> = [];
        let success = 0;
        let failed = 0;

        for (let i = 0; i < docs.length; i++) {
            const doc = docs[i]!;
            const primaryKey = this.extractPrimaryKey(doc);

            if (this.indexManager.hasByPrimaryKey(primaryKey)) {
                if (options.throwOnDuplicate) {
                    failed++;
                    errors.push({
                        index: i,
                        error: `Document with primary key "${primaryKey}" already exists`,
                    });
                    continue;
                }
                failed++;
                errors.push({
                    index: i,
                    error: `Duplicate key: ${primaryKey}`,
                });
                continue;
            }

            try {
                this.indexManager.insert(doc, true);
                insertedDocs.push(doc);
                success++;
            } catch (error) {
                failed++;
                errors.push({
                    index: i,
                    error: error instanceof Error ? error.message : "Unknown error",
                });
            }
        }

        if (insertedDocs.length > 0) {
            this.emitEvent("insert", { docs: insertedDocs });
        }

        const result: InsertManyResult<T> = {
            success: failed === 0,
            insertedCount: success,
            failedCount: failed,
            insertedDocs,
        };

        if (errors.length > 0) {
            result.errors = errors;
        }

        return result;
    }

    /**
     * 查找文档
     * @param options 查询选项
     */
    public find(options: QueryOptions<T> = {}): FindResult<T> {
        const builder = this.query();

        if (options.where) {
            builder.where(options.where);
        }

        if (options.sort) {
            for (const [field, direction] of Object.entries(options.sort)) {
                builder.sortBy(field as keyof T, direction);
            }
        }

        if (options.offset !== undefined && options.offset > 0) {
            builder.skip(options.offset);
        }

        if (options.limit !== undefined && options.limit > 0) {
            builder.limit(options.limit);
        }

        if (options.fields && options.fields.length > 0) {
            builder.select(...options.fields);
        }

        return builder.toArray();
    }

    /**
     * 查找单条文档
     * @param filter 查询条件
     */
    public findOne(filter: QueryFilter<T>): T | null {
        return this.query().where(filter).first();
    }

    /**
     * 通过主键查找文档
     * @param primaryKey 主键值
     */
    public findByPrimaryKey(primaryKey: PrimaryKeyType): T | undefined {
        return this.indexManager.findByPrimaryKey(primaryKey as IndexKey);
    }

    /**
     * 通过二级索引查找文档
     * @param indexName 索引名称
     * @param value 索引值
     */
    public findBySecondaryIndex(indexName: string, value: unknown): T[] {
        return this.indexManager.findBySecondaryIndex(indexName, value as IndexValue);
    }

    /**
     * 更新文档
     * @param filter 查询条件
     * @param update 更新操作
     * @param options 更新选项
     */
    public update(filter: QueryFilter<T>, update: UpdateOperation<T>, options: UpdateOptions = {}): UpdateResult {
        const docs = this.find({ where: filter }).docs;

        if (docs.length === 0) {
            if (options.upsert) {
                return this.upsert(filter, update);
            }

            if (options.throwOnMissing) {
                throw new CollectionError("No document matches the filter", "DOCUMENT_NOT_FOUND");
            }

            return {
                success: false,
                matchedCount: 0,
                modifiedCount: 0,
            };
        }

        const toUpdate = options.multi ? docs : docs.slice(0, 1);
        let modifiedCount = 0;

        for (const doc of toUpdate) {
            const updatedDoc = this.applyUpdate(doc, update);
            const oldDoc = { ...doc };

            if (this.indexManager.update(oldDoc, updatedDoc)) {
                modifiedCount++;
                this.emitEvent("update", { doc: updatedDoc, oldDoc });
            }
        }

        return {
            success: true,
            matchedCount: docs.length,
            modifiedCount,
        };
    }

    /**
     * 更新单条文档
     * @param filter 查询条件
     * @param update 更新操作
     * @param options 更新选项
     */
    public updateOne(filter: QueryFilter<T>, update: UpdateOperation<T>, options: Omit<UpdateOptions, "multi"> = {}): UpdateResult {
        return this.update(filter, update, { ...options, multi: false });
    }

    /**
     * 更新多条文档
     * @param filter 查询条件
     * @param update 更新操作
     * @param options 更新选项
     */
    public updateMany(filter: QueryFilter<T>, update: UpdateOperation<T>, options: Omit<UpdateOptions, "multi"> = {}): UpdateResult {
        return this.update(filter, update, { ...options, multi: true });
    }

    /**
     * Upsert 操作
     */
    private upsert(filter: QueryFilter<T>, update: UpdateOperation<T>): UpdateResult {
        const newDoc = this.createDocFromUpdate(filter, update);
        const primaryKey = this.extractPrimaryKey(newDoc);

        if (this.indexManager.hasByPrimaryKey(primaryKey)) {
            const existingDoc = this.indexManager.findByPrimaryKey(primaryKey)!;
            const updatedDoc = this.applyUpdate(existingDoc, update);

            this.indexManager.update(existingDoc, updatedDoc);
            this.emitEvent("update", { doc: updatedDoc, oldDoc: existingDoc });

            return {
                success: true,
                matchedCount: 1,
                modifiedCount: 1,
                upserted: false,
            };
        }

        this.indexManager.insert(newDoc, true);
        this.emitEvent("insert", { doc: newDoc });

        return {
            success: true,
            matchedCount: 0,
            modifiedCount: 0,
            upserted: true,
            upsertedId: primaryKey,
        };
    }

    /**
     * 删除文档
     * @param filter 查询条件
     * @param options 删除选项
     */
    public delete(filter: QueryFilter<T>, options: DeleteOptions = {}): DeleteResult {
        const docs = this.find({ where: filter }).docs;

        if (docs.length === 0) {
            if (options.throwOnMissing) {
                throw new CollectionError("No document matches the filter", "DOCUMENT_NOT_FOUND");
            }

            return {
                success: false,
                deletedCount: 0,
            };
        }

        const toDelete = options.multi ? docs : docs.slice(0, 1);
        let deletedCount = 0;

        for (const doc of toDelete) {
            if (this.indexManager.delete(doc)) {
                deletedCount++;
                this.emitEvent("delete", { doc });
            }
        }

        return {
            success: true,
            deletedCount,
        };
    }

    /**
     * 删除单条文档
     * @param filter 查询条件
     * @param options 删除选项
     */
    public deleteOne(filter: QueryFilter<T>, options: Omit<DeleteOptions, "multi"> = {}): DeleteResult {
        return this.delete(filter, { ...options, multi: false });
    }

    /**
     * 删除多条文档
     * @param filter 查询条件
     * @param options 删除选项
     */
    public deleteMany(filter: QueryFilter<T>, options: Omit<DeleteOptions, "multi"> = {}): DeleteResult {
        return this.delete(filter, { ...options, multi: true });
    }

    /**
     * 通过主键删除文档
     * @param primaryKey 主键值
     */
    public deleteByPrimaryKey(primaryKey: PrimaryKeyType): boolean {
        const doc = this.indexManager.findByPrimaryKey(primaryKey as IndexKey);
        if (!doc) {
            return false;
        }

        const result = this.indexManager.delete(doc);
        if (result) {
            this.emitEvent("delete", { doc });
        }
        return result !== undefined;
    }

    /**
     * 统计文档数量
     * @param filter 查询条件
     */
    public count(filter: QueryFilter<T> = {}): number {
        if (Object.keys(filter).length === 0) {
            return this.indexManager.Size;
        }
        return this.query().where(filter).count();
    }

    /**
     * 检查文档是否存在
     * @param filter 查询条件
     */
    public exists(filter: QueryFilter<T>): boolean {
        return this.query().where(filter).hasMatch();
    }

    /**
     * 创建查询构建器
     */
    public query(): QueryBuilder<T> {
        return new QueryBuilder<T>(this);
    }

    /**
     * 清空集合
     */
    public clear(): void {
        this.indexManager.clear();
    }

    /**
     * 获取集合统计信息
     */
    public getStats(): CollectionStats {
        const indexStats = this.indexManager.getStats();

        return {
            name: this.name,
            documentCount: this.indexManager.Size,
            indexCount: indexStats.length,
            indexes: indexStats.map((stat) => ({
                name: stat.name,
                type: stat.type === "primary" ? "primary" : "secondary",
                fieldName: stat.fieldName,
                unique: stat.unique,
                keyCount: stat.keyCount,
            })),
            memoryUsage: this.indexManager.getMemoryUsage(),
        };
    }

    /**
     * 创建二级索引
     * @param config 索引配置
     */
    public createIndex(config: IndexConfig<T>): void {
        this.indexManager.createSecondaryIndex({
            name: config.name,
            fieldName: config.fieldName,
            unique: config.unique ?? false,
            sparse: config.sparse ?? false,
            valueExtractor: (doc: T) => doc[config.fieldName as keyof T] as IndexValue,
        });
    }

    /**
     * 删除二级索引
     * @param name 索引名称
     */
    public dropIndex(name: string): boolean {
        return this.indexManager.dropSecondaryIndex(name);
    }

    /**
     * 获取所有索引名称
     */
    public getIndexNames(): string[] {
        return [this.primaryKeyField, ...this.indexManager.getSecondaryIndexNames()];
    }

    /**
     * 添加事件监听器
     * @param event 事件类型
     * @param listener 监听器函数
     */
    public on(event: CollectionEventType, listener: CollectionEventListener<T>): void {
        if (!this.eventListeners.has(event)) {
            this.eventListeners.set(event, new Set());
        }
        this.eventListeners.get(event)!.add(listener);
    }

    /**
     * 移除事件监听器
     * @param event 事件类型
     * @param listener 监听器函数
     */
    public off(event: CollectionEventType, listener: CollectionEventListener<T>): void {
        const listeners = this.eventListeners.get(event);
        if (listeners) {
            listeners.delete(listener);
        }
    }

    /**
     * 触发事件
     */
    private emitEvent(event: CollectionEventType, data: Parameters<CollectionEventListener<T>>[1]): void {
        const listeners = this.eventListeners.get(event);
        if (listeners) {
            for (const listener of listeners) {
                try {
                    listener(event, data);
                } catch {
                    // 忽略监听器错误
                }
            }
        }
    }

    /**
     * 提取主键值
     */
    public extractPrimaryKey(doc: T): PrimaryKeyType {
        const value = doc[this.primaryKeyField];
        if (value === undefined || value === null) {
            throw new CollectionError(`Primary key field "${this.primaryKeyField}" is required`, "MISSING_PRIMARY_KEY");
        }
        if (typeof value !== "string" && typeof value !== "number") {
            throw new CollectionError(`Primary key must be string or number, got ${typeof value}`, "INVALID_PRIMARY_KEY_TYPE");
        }
        return value;
    }

    /**
     * 应用更新操作
     */
    private applyUpdate(doc: T, update: UpdateOperation<T>): T {
        const updated = { ...doc };

        if (this.isUpdateOperators(update)) {
            return this.applyUpdateOperators(updated, update);
        }

        return { ...updated, ...update };
    }

    /**
     * 检查是否是更新操作符
     */
    private isUpdateOperators(update: UpdateOperation<T>): update is UpdateOperators<T> {
        if (typeof update !== "object" || update === null) {
            return false;
        }
        const keys = Object.keys(update);
        return keys.some((key) => key.startsWith("$"));
    }

    /**
     * 应用更新操作符
     */
    private applyUpdateOperators(doc: T, operators: UpdateOperators<T>): T {
        const result = { ...doc };

        if (operators.$set) {
            Object.assign(result, operators.$set);
        }

        if (operators.$unset) {
            for (const field of Object.keys(operators.$unset) as (keyof T)[]) {
                delete result[field];
            }
        }

        if (operators.$inc) {
            for (const [field, amount] of Object.entries(operators.$inc) as [keyof T, number][]) {
                const current = result[field];
                if (typeof current === "number") {
                    (result as Record<string, unknown>)[field as string] = current + amount;
                }
            }
        }

        if (operators.$mul) {
            for (const [field, multiplier] of Object.entries(operators.$mul) as [keyof T, number][]) {
                const current = result[field];
                if (typeof current === "number") {
                    (result as Record<string, unknown>)[field as string] = current * multiplier;
                }
            }
        }

        if (operators.$rename) {
            for (const [oldName, newName] of Object.entries(operators.$rename) as [keyof T, string][]) {
                if (oldName in result) {
                    (result as Record<string, unknown>)[newName] = result[oldName];
                    delete result[oldName];
                }
            }
        }

        if (operators.$currentDate) {
            const now = new Date();
            for (const field of Object.keys(operators.$currentDate) as (keyof T)[]) {
                const config = operators.$currentDate[field];
                if (config === true) {
                    (result as Record<string, unknown>)[field as string] = now;
                } else if (typeof config === "object") {
                    (result as Record<string, unknown>)[field as string] = config.$type === "timestamp" ? now.getTime() : now;
                }
            }
        }

        if (operators.$push) {
            for (const [field, value] of Object.entries(operators.$push) as [keyof T, unknown][]) {
                const current = result[field];
                if (Array.isArray(current)) {
                    if (typeof value === "object" && value !== null && "$each" in value) {
                        const eachValue = value as {
                            $each?: unknown[];
                            $position?: number;
                            $slice?: number;
                        };
                        if (eachValue.$each) {
                            if (eachValue.$position !== undefined) {
                                current.splice(eachValue.$position, 0, ...eachValue.$each);
                            } else {
                                current.push(...eachValue.$each);
                            }
                            if (eachValue.$slice !== undefined) {
                                if (eachValue.$slice < 0) {
                                    current.splice(0, current.length + eachValue.$slice);
                                } else if (eachValue.$slice > 0) {
                                    current.splice(eachValue.$slice);
                                }
                            }
                        }
                    } else {
                        current.push(value);
                    }
                }
            }
        }

        if (operators.$pull) {
            for (const [field, condition] of Object.entries(operators.$pull) as [keyof T, unknown][]) {
                const current = result[field];
                if (Array.isArray(current)) {
                    if (typeof condition === "object" && condition !== null) {
                        const filter = condition as QueryFilter<unknown>;
                        const indicesToRemove: number[] = [];
                        for (let i = 0; i < current.length; i++) {
                            if (QueryParser.matches(current[i], filter)) {
                                indicesToRemove.push(i);
                            }
                        }
                        for (let i = indicesToRemove.length - 1; i >= 0; i--) {
                            current.splice(indicesToRemove[i]!, 1);
                        }
                    } else {
                        const index = current.indexOf(condition);
                        if (index > -1) {
                            current.splice(index, 1);
                        }
                    }
                }
            }
        }

        if (operators.$addToSet) {
            for (const [field, value] of Object.entries(operators.$addToSet) as [keyof T, unknown][]) {
                const current = result[field];
                if (Array.isArray(current)) {
                    if (typeof value === "object" && value !== null && "$each" in value) {
                        const eachValue = value as { $each?: unknown[] };
                        if (eachValue.$each) {
                            for (const item of eachValue.$each) {
                                if (!current.includes(item)) {
                                    current.push(item);
                                }
                            }
                        }
                    } else {
                        if (!current.includes(value)) {
                            current.push(value);
                        }
                    }
                }
            }
        }

        if (operators.$pop) {
            for (const [field, direction] of Object.entries(operators.$pop) as [keyof T, 1 | -1][]) {
                const current = result[field];
                if (Array.isArray(current) && current.length > 0) {
                    if (direction === 1) {
                        current.pop();
                    } else {
                        current.shift();
                    }
                }
            }
        }

        return result;
    }

    /**
     * 从更新操作创建文档
     */
    private createDocFromUpdate(filter: QueryFilter<T>, update: UpdateOperation<T>): T {
        const doc: Record<string, unknown> = {};

        for (const [key, value] of Object.entries(filter)) {
            if (!key.startsWith("$") && typeof value !== "object") {
                doc[key] = value;
            }
        }

        if (this.isUpdateOperators(update)) {
            if (update.$set) {
                Object.assign(doc, update.$set);
            }
        } else {
            Object.assign(doc, update);
        }

        return doc as T;
    }

    /**
     * 获取所有文档
     */
    public getAll(): T[] {
        return this.indexManager.getAll();
    }

    /**
     * 获取主键字段名（实现 QueryExecutor 接口）
     */
    public getPrimaryKeyField(): string {
        return this.primaryKeyField;
    }

    /**
     * 获取二级索引字段列表（实现 QueryExecutor 接口）
     */
    public getSecondaryIndexFields(): string[] {
        return this.indexManager.getSecondaryIndexNames();
    }

    /**
     * 验证集合一致性
     */
    public validate(): { valid: boolean; errors: string[] } {
        const result = this.indexManager.validate();
        return {
            valid: result.valid,
            errors: result.errors.flatMap((e) => e.errors),
        };
    }

    /**
     * 重建所有索引
     */
    public rebuildIndexes(): void {
        const docs = this.getAll();
        this.indexManager.rebuild(docs);
    }
}
