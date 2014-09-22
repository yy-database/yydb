/**
 * LightDB 索引引擎错误定义
 * 提供详细的索引相关错误类型
 */

import { StorageError } from "../errors";

/**
 * 索引错误基类
 */
export class IndexError extends StorageError {
    constructor(message: string, code: string, context?: Record<string, unknown>) {
        super(message, code, context);
        this.name = "IndexError";
        Object.setPrototypeOf(this, IndexError.prototype);
    }
}

/**
 * 重复键错误
 */
export class DuplicateKeyError extends IndexError {
    constructor(indexName: string, key: unknown) {
        super(`Duplicate key '${String(key)}' in unique index '${indexName}'`, "DUPLICATE_KEY", { indexName, key });
        this.name = "DuplicateKeyError";
        Object.setPrototypeOf(this, DuplicateKeyError.prototype);
    }
}

/**
 * 键未找到错误
 */
export class KeyNotFoundError extends IndexError {
    constructor(indexName: string, key: unknown) {
        super(`Key '${String(key)}' not found in index '${indexName}'`, "KEY_NOT_FOUND", { indexName, key });
        this.name = "KeyNotFoundError";
        Object.setPrototypeOf(this, KeyNotFoundError.prototype);
    }
}

/**
 * 索引不存在错误
 */
export class IndexNotFoundError extends IndexError {
    constructor(indexName: string) {
        super(`Index '${indexName}' not found`, "INDEX_NOT_FOUND", { indexName });
        this.name = "IndexNotFoundError";
        Object.setPrototypeOf(this, IndexNotFoundError.prototype);
    }
}

/**
 * 索引已存在错误
 */
export class IndexExistsError extends IndexError {
    constructor(indexName: string) {
        super(`Index '${indexName}' already exists`, "INDEX_EXISTS", { indexName });
        this.name = "IndexExistsError";
        Object.setPrototypeOf(this, IndexExistsError.prototype);
    }
}

/**
 * 索引配置错误
 */
export class IndexConfigError extends IndexError {
    constructor(message: string, option?: string, value?: unknown) {
        super(message, "INDEX_CONFIG_ERROR", { option, value });
        this.name = "IndexConfigError";
        Object.setPrototypeOf(this, IndexConfigError.prototype);
    }
}

/**
 * 索引操作错误
 */
export class IndexOperationError extends IndexError {
    constructor(message: string, operation: string, indexName?: string) {
        super(message, "INDEX_OPERATION_ERROR", { operation, indexName });
        this.name = "IndexOperationError";
        Object.setPrototypeOf(this, IndexOperationError.prototype);
    }
}

/**
 * 索引一致性错误
 */
export class IndexConsistencyError extends IndexError {
    constructor(message: string, details?: unknown) {
        super(message, "INDEX_CONSISTENCY_ERROR", { details });
        this.name = "IndexConsistencyError";
        Object.setPrototypeOf(this, IndexConsistencyError.prototype);
    }
}

/**
 * 范围查询错误
 */
export class RangeQueryError extends IndexError {
    constructor(message: string, range?: { start?: unknown; end?: unknown }) {
        super(message, "RANGE_QUERY_ERROR", { range });
        this.name = "RangeQueryError";
        Object.setPrototypeOf(this, RangeQueryError.prototype);
    }
}
