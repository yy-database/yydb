/**
 * 惰性求值查询实现
 * 实现 Iterable 风格的惰性求值，不一次性加载所有结果
 */

import { LazyQueryOptions, LazyQueryStats } from "./types";

/**
 * 过滤函数类型
 */
type FilterFunction<T> = (item: T, index: number) => boolean;

/**
 * 映射函数类型
 */
type MapFunction<T, R> = (item: T, index: number) => R;

/**
 * 转换函数类型
 */
type TransformFunction<T> = (item: T) => T;

/**
 * 惰性查询迭代器
 * 支持链式操作和提前终止
 */
export class LazyQueryIterator<T> implements IterableIterator<T> {
    private source: Iterator<T> | Iterable<T>;
    private filters: FilterFunction<T>[];
    private transforms: TransformFunction<T>[];
    private currentIndex: number;
    private limitValue: number;
    private skipValue: number;
    private yieldedCount: number;
    private completed: boolean;

    /**
     * 创建惰性查询迭代器
     * @param source 数据源
     * @param options 配置选项
     */
    constructor(source: Iterator<T> | Iterable<T>, _options: LazyQueryOptions = {}) {
        this.source = source;
        this.filters = [];
        this.transforms = [];
        this.currentIndex = 0;
        this.limitValue = 0;
        this.skipValue = 0;
        this.yieldedCount = 0;
        this.completed = false;
    }

    /**
     * 添加过滤条件
     * @param predicate 过滤函数
     */
    public filter(predicate: FilterFunction<T>): this {
        this.filters.push(predicate);
        return this;
    }

    /**
     * 添加映射转换
     * @param mapper 映射函数
     */
    public map<R>(mapper: MapFunction<T, R>): LazyQueryIterator<R> {
        const newIterator = new LazyQueryIterator<R>(this as unknown as Iterable<R>);
        (newIterator as unknown as { originalMap: MapFunction<T, R> }).originalMap = mapper;
        return newIterator;
    }

    /**
     * 添加转换
     * @param transform 转换函数
     */
    public transform(transform: TransformFunction<T>): this {
        this.transforms.push(transform);
        return this;
    }

    /**
     * 设置限制
     * @param count 最大数量
     */
    public limit(count: number): this {
        this.limitValue = Math.max(0, count);
        return this;
    }

    /**
     * 设置跳过
     * @param count 跳过数量
     */
    public skip(count: number): this {
        this.skipValue = Math.max(0, count);
        return this;
    }

    /**
     * 获取迭代器
     */
    public [Symbol.iterator](): IterableIterator<T> {
        return this;
    }

    /**
     * 获取下一个元素
     */
    public next(): IteratorResult<T> {
        if (this.completed) {
            return { done: true, value: undefined };
        }

        if (this.limitValue > 0 && this.yieldedCount >= this.limitValue) {
            this.completed = true;
            return { done: true, value: undefined };
        }

        const iterator = this.getIterator();

        while (true) {
            const result = iterator.next();

            if (result.done) {
                this.completed = true;
                return { done: true, value: undefined };
            }

            let value = result.value;
            this.currentIndex++;

            for (const transform of this.transforms) {
                value = transform(value);
            }

            let passes = true;
            for (const filter of this.filters) {
                if (!filter(value, this.currentIndex - 1)) {
                    passes = false;
                    break;
                }
            }

            if (!passes) {
                continue;
            }

            if (this.skipValue > 0) {
                this.skipValue--;
                continue;
            }

            this.yieldedCount++;
            return { done: false, value };
        }
    }

    /**
     * 获取底层迭代器
     */
    private getIterator(): Iterator<T> {
        if (Symbol.iterator in this.source) {
            return (this.source as Iterable<T>)[Symbol.iterator]();
        }
        return this.source as Iterator<T>;
    }

    /**
     * 转换为数组
     */
    public toArray(): T[] {
        const result: T[] = [];
        for (const item of this) {
            result.push(item);
        }
        return result;
    }

    /**
     * 获取第一个元素
     */
    public first(): T | undefined {
        for (const item of this) {
            return item;
        }
        return undefined;
    }

    /**
     * 获取最后一个元素
     */
    public last(): T | undefined {
        let lastItem: T | undefined;
        for (const item of this) {
            lastItem = item;
        }
        return lastItem;
    }

    /**
     * 统计数量
     */
    public count(): number {
        let count = 0;
        for (const _ of this) {
            count++;
        }
        return count;
    }

    /**
     * 检查是否存在匹配元素
     */
    public some(predicate?: FilterFunction<T>): boolean {
        if (predicate) {
            this.filter(predicate);
        }
        for (const _ of this) {
            return true;
        }
        return false;
    }

    /**
     * 检查是否所有元素都匹配
     */
    public every(predicate: FilterFunction<T>): boolean {
        for (const item of this) {
            if (!predicate(item, this.currentIndex - 1)) {
                return false;
            }
        }
        return true;
    }

    /**
     * 查找匹配元素
     */
    public find(predicate: FilterFunction<T>): T | undefined {
        this.filter(predicate);
        return this.first();
    }

    /**
     * 查找匹配元素的索引
     */
    public findIndex(predicate: FilterFunction<T>): number {
        let index = 0;
        for (const item of this) {
            if (predicate(item, index)) {
                return index;
            }
            index++;
        }
        return -1;
    }

    /**
     * 遍历所有元素
     * @param callback 回调函数
     */
    public forEach(callback: (item: T, index: number) => void): void {
        let index = 0;
        for (const item of this) {
            callback(item, index++);
        }
    }

    /**
     * 归约
     * @param reducer 归约函数
     * @param initial 初始值
     */
    public reduce<R>(reducer: (acc: R, item: T, index: number) => R, initial: R): R {
        let acc = initial;
        let index = 0;
        for (const item of this) {
            acc = reducer(acc, item, index++);
        }
        return acc;
    }

    /**
     * 获取统计信息
     */
    public getStats(): LazyQueryStats {
        return {
            iteratedCount: this.yieldedCount,
            completed: this.completed,
            usedIndex: false,
        };
    }

    /**
     * 重置迭代器
     */
    public reset(): void {
        this.currentIndex = 0;
        this.yieldedCount = 0;
        this.completed = false;
    }
}

/**
 * 惰性查询构建器
 * 提供链式 API 构建惰性查询
 */
export class LazyQueryBuilder<T> {
    private source: Iterable<T>;
    private filters: FilterFunction<T>[];
    private transforms: TransformFunction<T>[];
    private limitValue: number;
    private skipValue: number;
    private prefetchSize: number;

    constructor(source: Iterable<T>) {
        this.source = source;
        this.filters = [];
        this.transforms = [];
        this.limitValue = 0;
        this.skipValue = 0;
        this.prefetchSize = 0;
    }

    /**
     * 添加过滤条件
     * @param predicate 过滤函数
     */
    public where(predicate: FilterFunction<T>): this {
        this.filters.push(predicate);
        return this;
    }

    /**
     * 添加等于条件
     * @param field 字段名
     * @param value 值
     */
    public eq<K extends keyof T>(field: K, value: T[K]): this {
        return this.where((item) => item[field] === value);
    }

    /**
     * 添加不等于条件
     * @param field 字段名
     * @param value 值
     */
    public ne<K extends keyof T>(field: K, value: T[K]): this {
        return this.where((item) => item[field] !== value);
    }

    /**
     * 添加大于条件
     * @param field 字段名
     * @param value 值
     */
    public gt<K extends keyof T>(field: K, value: T[K]): this {
        return this.where((item) => (item[field] as unknown as number) > (value as unknown as number));
    }

    /**
     * 添加大于等于条件
     * @param field 字段名
     * @param value 值
     */
    public gte<K extends keyof T>(field: K, value: T[K]): this {
        return this.where((item) => (item[field] as unknown as number) >= (value as unknown as number));
    }

    /**
     * 添加小于条件
     * @param field 字段名
     * @param value 值
     */
    public lt<K extends keyof T>(field: K, value: T[K]): this {
        return this.where((item) => (item[field] as unknown as number) < (value as unknown as number));
    }

    /**
     * 添加小于等于条件
     * @param field 字段名
     * @param value 值
     */
    public lte<K extends keyof T>(field: K, value: T[K]): this {
        return this.where((item) => (item[field] as unknown as number) <= (value as unknown as number));
    }

    /**
     * 添加包含条件
     * @param field 字段名
     * @param values 值数组
     */
    public in<K extends keyof T>(field: K, values: T[K][]): this {
        const valueSet = new Set(values);
        return this.where((item) => valueSet.has(item[field]));
    }

    /**
     * 添加转换
     * @param transform 转换函数
     */
    public apply(transform: TransformFunction<T>): this {
        this.transforms.push(transform);
        return this;
    }

    /**
     * 设置限制
     * @param count 最大数量
     */
    public limit(count: number): this {
        this.limitValue = Math.max(0, count);
        return this;
    }

    /**
     * 设置跳过
     * @param count 跳过数量
     */
    public skip(count: number): this {
        this.skipValue = Math.max(0, count);
        return this;
    }

    /**
     * 设置预取大小
     * @param size 预取大小
     */
    public prefetch(size: number): this {
        this.prefetchSize = Math.max(0, size);
        return this;
    }

    /**
     * 构建迭代器
     */
    public build(): LazyQueryIterator<T> {
        const iterator = new LazyQueryIterator<T>(this.source, {
            prefetchSize: this.prefetchSize,
        });

        for (const filter of this.filters) {
            iterator.filter(filter);
        }

        for (const transform of this.transforms) {
            iterator.transform(transform);
        }

        if (this.limitValue > 0) {
            iterator.limit(this.limitValue);
        }

        if (this.skipValue > 0) {
            iterator.skip(this.skipValue);
        }

        return iterator;
    }

    /**
     * 执行查询并返回迭代器
     */
    public [Symbol.iterator](): IterableIterator<T> {
        return this.build();
    }

    /**
     * 转换为数组
     */
    public toArray(): T[] {
        return this.build().toArray();
    }

    /**
     * 获取第一个元素
     */
    public first(): T | undefined {
        return this.build().first();
    }

    /**
     * 获取最后一个元素
     */
    public last(): T | undefined {
        return this.build().last();
    }

    /**
     * 统计数量
     */
    public count(): number {
        return this.build().count();
    }

    /**
     * 遍历所有元素
     */
    public forEach(callback: (item: T, index: number) => void): void {
        this.build().forEach(callback);
    }
}

/**
 * 创建惰性查询
 * @param source 数据源
 */
export function lazy<T>(source: Iterable<T>): LazyQueryBuilder<T> {
    return new LazyQueryBuilder(source);
}

/**
 * 创建惰性查询迭代器
 * @param source 数据源
 */
export function lazyIterator<T>(source: Iterable<T>): LazyQueryIterator<T> {
    return new LazyQueryIterator(source);
}

/**
 * 生成器工具函数
 */
export class GeneratorUtils {
    /**
     * 创建范围生成器
     * @param start 起始值
     * @param end 结束值
     * @param step 步长
     */
    public static *range(start: number, end: number, step: number = 1): Generator<number> {
        if (step > 0) {
            for (let i = start; i < end; i += step) {
                yield i;
            }
        } else if (step < 0) {
            for (let i = start; i > end; i += step) {
                yield i;
            }
        }
    }

    /**
     * 创建重复生成器
     * @param value 值
     * @param count 重复次数，-1 表示无限
     */
    public static *repeat<T>(value: T, count: number = -1): Generator<T> {
        if (count < 0) {
            while (true) {
                yield value;
            }
        } else {
            for (let i = 0; i < count; i++) {
                yield value;
            }
        }
    }

    /**
     * 合并多个生成器
     * @param sources 生成器数组
     */
    public static *concat<T>(...sources: Iterable<T>[]): Generator<T, void, undefined> {
        for (const source of sources) {
            yield* source;
        }
    }

    /**
     * 交替生成
     * @param sources 生成器数组
     */
    public static *zip<T>(...sources: Iterable<T>[]): Generator<T[]> {
        const iterators = sources.map((s) => s[Symbol.iterator]());

        while (true) {
            const results: T[] = [];
            let done = true;

            for (const iterator of iterators) {
                const result = iterator.next();
                if (!result.done) {
                    results.push(result.value);
                    done = false;
                }
            }

            if (done) {
                break;
            }

            yield results;
        }
    }

    /**
     * 展开嵌套生成器
     * @param source 嵌套生成器
     */
    public static *flatten<T>(source: Iterable<Iterable<T>>): Generator<T, void, undefined> {
        for (const inner of source) {
            yield* inner;
        }
    }

    /**
     * 带索引生成
     * @param source 生成器
     */
    public static *enumerate<T>(source: Iterable<T>): Generator<[number, T]> {
        let index = 0;
        for (const item of source) {
            yield [index++, item];
        }
    }

    /**
     * 分块生成
     * @param source 生成器
     * @param size 块大小
     */
    public static *chunk<T>(source: Iterable<T>, size: number): Generator<T[]> {
        let chunk: T[] = [];

        for (const item of source) {
            chunk.push(item);
            if (chunk.length >= size) {
                yield chunk;
                chunk = [];
            }
        }

        if (chunk.length > 0) {
            yield chunk;
        }
    }

    /**
     * 去重生成
     * @param source 生成器
     * @param keySelector 键选择器
     */
    public static *distinct<T, K = T>(source: Iterable<T>, keySelector?: (item: T) => K): Generator<T> {
        const seen = new Set<K>();
        const selector = keySelector ?? ((item: T) => item as unknown as K);

        for (const item of source) {
            const key = selector(item);
            if (!seen.has(key)) {
                seen.add(key);
                yield item;
            }
        }
    }

    /**
     * 取前 N 个
     * @param source 生成器
     * @param count 数量
     */
    public static *take<T>(source: Iterable<T>, count: number): Generator<T> {
        let taken = 0;
        for (const item of source) {
            if (taken >= count) {
                break;
            }
            yield item;
            taken++;
        }
    }

    /**
     * 跳过前 N 个
     * @param source 生成器
     * @param count 数量
     */
    public static *skip<T>(source: Iterable<T>, count: number): Generator<T> {
        let skipped = 0;
        for (const item of source) {
            if (skipped >= count) {
                yield item;
            } else {
                skipped++;
            }
        }
    }
}
