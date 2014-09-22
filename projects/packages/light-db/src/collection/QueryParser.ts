/**
 * LightDB 查询条件解析器
 * 解析查询条件并判断文档是否匹配
 */

import type {
    QueryFilter,
    ComparisonOperators,
    ArrayOperators,
    ExistenceOperators,
    RegexOperators,
    LogicalOperators,
    PrimaryKeyType,
} from "./types";

/**
 * 查询解析结果
 */
export interface QueryParseResult {
    /** 是否可以使用索引 */
    canUseIndex: boolean;
    /** 可以使用的索引字段 */
    indexField?: string;
    /** 索引查询值 */
    indexValue?: PrimaryKeyType | PrimaryKeyType[];
    /** 是否是精确匹配 */
    isExactMatch: boolean;
}

/**
 * 查询条件解析器
 * 提供查询条件的解析和匹配功能
 */
export class QueryParser {
    /**
     * 检查文档是否匹配查询条件
     * @param doc 文档
     * @param filter 查询条件
     */
    public static matches<T>(doc: T, filter: QueryFilter<T>): boolean {
        if (!filter || Object.keys(filter).length === 0) {
            return true;
        }

        const logicalOperators = ["$and", "$or", "$not", "$nor"] as const;
        const hasLogicalOperators = logicalOperators.some((op) => op in filter);

        if (hasLogicalOperators) {
            return this.matchesLogicalOperators(doc, filter);
        }

        for (const [field, condition] of Object.entries(filter)) {
            if (logicalOperators.includes(field as (typeof logicalOperators)[number])) {
                continue;
            }

            if (!this.matchesField(doc, field, condition)) {
                return false;
            }
        }

        return true;
    }

    /**
     * 匹配逻辑操作符
     */
    private static matchesLogicalOperators<T>(doc: T, filter: QueryFilter<T>): boolean {
        const logicalFilter = filter as LogicalOperators<T> & QueryFilter<T>;

        if (logicalFilter.$and) {
            if (!logicalFilter.$and.every((f) => this.matches(doc, f))) {
                return false;
            }
        }

        if (logicalFilter.$or) {
            if (!logicalFilter.$or.some((f) => this.matches(doc, f))) {
                return false;
            }
        }

        if (logicalFilter.$not) {
            if (this.matches(doc, logicalFilter.$not)) {
                return false;
            }
        }

        if (logicalFilter.$nor) {
            if (logicalFilter.$nor.some((f) => this.matches(doc, f))) {
                return false;
            }
        }

        const logicalOperators = ["$and", "$or", "$not", "$nor"] as const;
        for (const [field, condition] of Object.entries(filter)) {
            if (logicalOperators.includes(field as (typeof logicalOperators)[number])) {
                continue;
            }

            if (!this.matchesField(doc, field, condition)) {
                return false;
            }
        }

        return true;
    }

    /**
     * 匹配单个字段
     */
    private static matchesField<T>(doc: T, field: string, condition: unknown): boolean {
        const value = this.getNestedValue(doc, field);

        if (condition === null || condition === undefined) {
            return value === condition;
        }

        if (this.isComparisonOperators(condition)) {
            return this.matchesComparisonOperators(value, condition);
        }

        if (this.isArrayOperators(condition)) {
            return this.matchesArrayOperators(value, condition);
        }

        if (this.isExistenceOperators(condition)) {
            return this.matchesExistenceOperators(value, condition);
        }

        if (this.isRegexOperators(condition)) {
            return this.matchesRegexOperators(value, condition);
        }

        return this.isEqual(value, condition);
    }

    /**
     * 检查是否是比较操作符
     */
    private static isComparisonOperators(condition: unknown): condition is ComparisonOperators<unknown> {
        if (typeof condition !== "object" || condition === null) {
            return false;
        }

        const comparisonKeys = ["$eq", "$ne", "$gt", "$gte", "$lt", "$lte"];
        return Object.keys(condition).some((key) => comparisonKeys.includes(key));
    }

    /**
     * 检查是否是数组操作符
     */
    private static isArrayOperators(condition: unknown): condition is ArrayOperators<unknown> {
        if (typeof condition !== "object" || condition === null) {
            return false;
        }

        return "$in" in condition || "$nin" in condition;
    }

    /**
     * 检查是否是存在性操作符
     */
    private static isExistenceOperators(condition: unknown): condition is ExistenceOperators {
        if (typeof condition !== "object" || condition === null) {
            return false;
        }

        return "$exists" in condition;
    }

    /**
     * 检查是否是正则表达式操作符
     */
    private static isRegexOperators(condition: unknown): condition is RegexOperators {
        if (typeof condition !== "object" || condition === null) {
            return false;
        }

        return "$regex" in condition;
    }

    /**
     * 匹配比较操作符
     */
    private static matchesComparisonOperators(value: unknown, condition: ComparisonOperators<unknown>): boolean {
        if ("$eq" in condition && condition.$eq !== undefined) {
            if (!this.isEqual(value, condition.$eq)) {
                return false;
            }
        }

        if ("$ne" in condition && condition.$ne !== undefined) {
            if (this.isEqual(value, condition.$ne)) {
                return false;
            }
        }

        if ("$gt" in condition && condition.$gt !== undefined) {
            if (!this.isGreaterThan(value, condition.$gt)) {
                return false;
            }
        }

        if ("$gte" in condition && condition.$gte !== undefined) {
            if (!this.isGreaterThanOrEqual(value, condition.$gte)) {
                return false;
            }
        }

        if ("$lt" in condition && condition.$lt !== undefined) {
            if (!this.isLessThan(value, condition.$lt)) {
                return false;
            }
        }

        if ("$lte" in condition && condition.$lte !== undefined) {
            if (!this.isLessThanOrEqual(value, condition.$lte)) {
                return false;
            }
        }

        return true;
    }

    /**
     * 匹配数组操作符
     */
    private static matchesArrayOperators(value: unknown, condition: ArrayOperators<unknown>): boolean {
        if ("$in" in condition && condition.$in !== undefined) {
            if (!condition.$in.some((v) => this.isEqual(value, v))) {
                return false;
            }
        }

        if ("$nin" in condition && condition.$nin !== undefined) {
            if (condition.$nin.some((v) => this.isEqual(value, v))) {
                return false;
            }
        }

        return true;
    }

    /**
     * 匹配存在性操作符
     */
    private static matchesExistenceOperators(value: unknown, condition: ExistenceOperators): boolean {
        const exists = value !== undefined;

        if (condition.$exists === true) {
            return exists;
        }

        if (condition.$exists === false) {
            return !exists;
        }

        return true;
    }

    /**
     * 匹配正则表达式操作符
     */
    private static matchesRegexOperators(value: unknown, condition: RegexOperators): boolean {
        if (typeof value !== "string") {
            return false;
        }

        let regex: RegExp;

        if (condition.$regex instanceof RegExp) {
            regex = condition.$regex;
        } else if (typeof condition.$regex === "string") {
            const flags = condition.$options ?? "";
            regex = new RegExp(condition.$regex, flags);
        } else {
            return false;
        }

        return regex.test(value);
    }

    /**
     * 获取嵌套值
     */
    private static getNestedValue(obj: unknown, path: string): unknown {
        if (obj === null || obj === undefined) {
            return undefined;
        }

        const parts = path.split(".");
        let current: unknown = obj;

        for (const part of parts) {
            if (current === null || current === undefined) {
                return undefined;
            }

            if (typeof current === "object") {
                current = (current as Record<string, unknown>)[part];
            } else {
                return undefined;
            }
        }

        return current;
    }

    /**
     * 判断两个值是否相等
     */
    private static isEqual(a: unknown, b: unknown): boolean {
        if (a === b) {
            return true;
        }

        if (a === null || b === null) {
            return a === b;
        }

        if (typeof a !== typeof b) {
            return false;
        }

        if (Array.isArray(a) && Array.isArray(b)) {
            if (a.length !== b.length) {
                return false;
            }
            return a.every((v, i) => this.isEqual(v, b[i]));
        }

        if (typeof a === "object" && typeof b === "object") {
            const aKeys = Object.keys(a as object);
            const bKeys = Object.keys(b as object);

            if (aKeys.length !== bKeys.length) {
                return false;
            }

            return aKeys.every((key) => this.isEqual((a as Record<string, unknown>)[key], (b as Record<string, unknown>)[key]));
        }

        return false;
    }

    /**
     * 判断是否大于
     */
    private static isGreaterThan(a: unknown, b: unknown): boolean {
        if (typeof a === "number" && typeof b === "number") {
            return a > b;
        }

        if (typeof a === "string" && typeof b === "string") {
            return a > b;
        }

        if (a instanceof Date && b instanceof Date) {
            return a > b;
        }

        return false;
    }

    /**
     * 判断是否大于等于
     */
    private static isGreaterThanOrEqual(a: unknown, b: unknown): boolean {
        if (typeof a === "number" && typeof b === "number") {
            return a >= b;
        }

        if (typeof a === "string" && typeof b === "string") {
            return a >= b;
        }

        if (a instanceof Date && b instanceof Date) {
            return a >= b;
        }

        return false;
    }

    /**
     * 判断是否小于
     */
    private static isLessThan(a: unknown, b: unknown): boolean {
        if (typeof a === "number" && typeof b === "number") {
            return a < b;
        }

        if (typeof a === "string" && typeof b === "string") {
            return a < b;
        }

        if (a instanceof Date && b instanceof Date) {
            return a < b;
        }

        return false;
    }

    /**
     * 判断是否小于等于
     */
    private static isLessThanOrEqual(a: unknown, b: unknown): boolean {
        if (typeof a === "number" && typeof b === "number") {
            return a <= b;
        }

        if (typeof a === "string" && typeof b === "string") {
            return a <= b;
        }

        if (a instanceof Date && b instanceof Date) {
            return a <= b;
        }

        return false;
    }

    /**
     * 分析查询条件，判断是否可以使用索引
     * @param filter 查询条件
     * @param indexFields 可用的索引字段列表
     */
    public static analyzeQuery<T>(filter: QueryFilter<T>, indexFields: string[]): QueryParseResult {
        if (!filter || Object.keys(filter).length === 0) {
            return {
                canUseIndex: false,
                isExactMatch: false,
            };
        }

        const logicalOperators = ["$and", "$or", "$not", "$nor"];

        for (const field of indexFields) {
            const condition = (filter as Record<string, unknown>)[field];

            if (condition !== undefined && !logicalOperators.includes(field)) {
                if (this.isExactValue(condition)) {
                    return {
                        canUseIndex: true,
                        indexField: field,
                        indexValue: condition as PrimaryKeyType,
                        isExactMatch: true,
                    };
                }

                if (this.isInOperator(condition)) {
                    const inCondition = condition as ArrayOperators<unknown>;
                    return {
                        canUseIndex: true,
                        indexField: field,
                        indexValue: inCondition.$in as PrimaryKeyType[],
                        isExactMatch: false,
                    };
                }

                if (this.hasComparisonOperators(condition)) {
                    return {
                        canUseIndex: true,
                        indexField: field,
                        isExactMatch: false,
                    };
                }
            }
        }

        if (filter.$and) {
            for (const subFilter of filter.$and) {
                const result = this.analyzeQuery(subFilter, indexFields);
                if (result.canUseIndex && result.isExactMatch) {
                    return result;
                }
            }
        }

        return {
            canUseIndex: false,
            isExactMatch: false,
        };
    }

    /**
     * 检查是否是精确值
     */
    private static isExactValue(condition: unknown): boolean {
        if (condition === null || condition === undefined) {
            return true;
        }

        if (typeof condition === "object") {
            const keys = Object.keys(condition);
            const operatorKeys = keys.filter((k) => k.startsWith("$"));
            return operatorKeys.length === 0;
        }

        return true;
    }

    /**
     * 检查是否是 $in 操作符
     */
    private static isInOperator(condition: unknown): boolean {
        if (typeof condition === "object" && condition !== null) {
            return "$in" in condition && Array.isArray((condition as ArrayOperators<unknown>).$in);
        }
        return false;
    }

    /**
     * 检查是否有比较操作符
     */
    private static hasComparisonOperators(condition: unknown): boolean {
        if (typeof condition === "object" && condition !== null) {
            const comparisonKeys = ["$eq", "$ne", "$gt", "$gte", "$lt", "$lte"];
            return Object.keys(condition).some((key) => comparisonKeys.includes(key));
        }
        return false;
    }

    /**
     * 提取查询条件中的字段列表
     */
    public static extractFields<T>(filter: QueryFilter<T>): string[] {
        const fields: string[] = [];
        const logicalOperators = ["$and", "$or", "$not", "$nor"];

        for (const field of Object.keys(filter)) {
            if (!logicalOperators.includes(field) && !field.startsWith("$")) {
                fields.push(field);
            }
        }

        if (filter.$and) {
            for (const subFilter of filter.$and) {
                fields.push(...this.extractFields(subFilter));
            }
        }

        if (filter.$or) {
            for (const subFilter of filter.$or) {
                fields.push(...this.extractFields(subFilter));
            }
        }

        return [...new Set(fields)];
    }
}
