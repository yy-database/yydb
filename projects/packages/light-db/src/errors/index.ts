/**
 * LightDB 存储引擎错误定义
 * 提供详细的错误类型和错误消息
 */

/**
 * 存储引擎错误基类
 */
export class StorageError extends Error {
    /** 错误代码 */
    public readonly code: string;

    /** 错误上下文信息 */
    public readonly context?: Record<string, unknown>;

    constructor(message: string, code: string, context?: Record<string, unknown>) {
        super(message);
        this.name = "StorageError";
        this.code = code;
        if (context !== undefined) {
            this.context = context;
        }
        Object.setPrototypeOf(this, StorageError.prototype);
    }
}

/**
 * 文件操作错误
 */
export class FileOperationError extends StorageError {
    constructor(message: string, filePath: string, operation: string) {
        super(message, "FILE_OPERATION_ERROR", { filePath, operation });
        this.name = "FileOperationError";
        Object.setPrototypeOf(this, FileOperationError.prototype);
    }
}

/**
 * 文件格式错误
 */
export class FileFormatError extends StorageError {
    constructor(message: string, filePath: string, expected?: string, actual?: unknown) {
        super(message, "FILE_FORMAT_ERROR", { filePath, expected, actual });
        this.name = "FileFormatError";
        Object.setPrototypeOf(this, FileFormatError.prototype);
    }
}

/**
 * 校验和错误
 */
export class ChecksumError extends StorageError {
    constructor(message: string, expected: number, actual: number, position?: number) {
        super(message, "CHECKSUM_ERROR", { expected, actual, position });
        this.name = "ChecksumError";
        Object.setPrototypeOf(this, ChecksumError.prototype);
    }
}

/**
 * WAL 错误
 */
export class WalError extends StorageError {
    constructor(message: string, lsn?: bigint, offset?: number) {
        super(message, "WAL_ERROR", { lsn: lsn?.toString(), offset });
        this.name = "WALError";
        Object.setPrototypeOf(this, WalError.prototype);
    }
}

/**
 * 恢复错误
 */
export class RecoveryError extends StorageError {
    constructor(message: string, phase: string, details?: unknown) {
        super(message, "RECOVERY_ERROR", { phase, details });
        this.name = "RecoveryError";
        Object.setPrototypeOf(this, RecoveryError.prototype);
    }
}

/**
 * 检查点错误
 */
export class CheckpointError extends StorageError {
    constructor(message: string, lsn?: bigint) {
        super(message, "CHECKPOINT_ERROR", { lsn: lsn?.toString() });
        this.name = "CheckpointError";
        Object.setPrototypeOf(this, CheckpointError.prototype);
    }
}

/**
 * 页面错误
 */
export class PageError extends StorageError {
    constructor(message: string, pageId: number, details?: unknown) {
        super(message, "PAGE_ERROR", { pageId, details });
        this.name = "PageError";
        Object.setPrototypeOf(this, PageError.prototype);
    }
}

/**
 * 序列化错误
 */
export class SerializationError extends StorageError {
    constructor(message: string, data?: unknown) {
        super(message, "SERIALIZATION_ERROR", { data });
        this.name = "SerializationError";
        Object.setPrototypeOf(this, SerializationError.prototype);
    }
}

/**
 * 配置错误
 */
export class ConfigurationError extends StorageError {
    constructor(message: string, option?: string, value?: unknown) {
        super(message, "CONFIGURATION_ERROR", { option, value });
        this.name = "ConfigurationError";
        Object.setPrototypeOf(this, ConfigurationError.prototype);
    }
}

/**
 * 与 `@yydb/yydb` 共用的稳定错误码
 *
 * 浏览器产品与 Node 产品对同一失败必须给出相同标识，宿主只按 `code` 分支。
 */
export const LightDbErrorCodes = {
    /** I/O 失败 */
    io: "yydb.io",
    /** 持久化数据损坏 */
    corrupt: "yydb.corrupt",
    /** Schema 文档与已持久化内容冲突 */
    schemaConflict: "yydb.schema.conflict",
    /** Schema 文档不合法 */
    schemaInvalid: "yydb.schema.invalid",
    /** 宿主不支持该能力 */
    unsupported: "yydb.unsupported",
    /** 会话或数据库已关闭 */
    sessionClosed: "yydb.session.closed",
} as const;

/**
 * 数据库错误选项
 */
export interface LightDbErrorOptions {
    /** 失败的操作（`schemaGet`、`kvPut` 等） */
    operation?: string;
    /** 底层失败 */
    cause?: unknown;
}

/**
 * 携带稳定错误码的数据库错误
 * 与 `@yydb/yydb` 的作者面同构：`code` 稳定，`message` 只用于显示。
 */
export class LightDbError extends Error {
    /** 稳定错误码 */
    public readonly code: string;

    /** 失败的操作 */
    public readonly operation: string;

    /** 是否可以用相同参数重试 */
    public readonly retryable: boolean;

    constructor(code: string, message: string, options: LightDbErrorOptions = {}) {
        super(message, { cause: options.cause });
        this.name = "LightDbError";
        this.code = code;
        this.operation = options.operation ?? "unknown";
        this.retryable = false;
        Object.setPrototypeOf(this, LightDbError.prototype);
    }
}
