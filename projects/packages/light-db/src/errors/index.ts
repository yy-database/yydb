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
