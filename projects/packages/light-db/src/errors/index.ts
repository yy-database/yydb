/**
 * 稳定错误模型
 *
 * 与 `@yydb/yydb` / `@yydb/yydb-client` 共用同一组错误码：宿主只按 `code` 分支，
 * `message` 只用于显示。
 */

/** 浏览器产品对外的稳定错误码 */
export const LightDbErrorCodes = {
    /** I/O 失败 */
    io: 'yydb.io',
    /** 持久化数据损坏 */
    corrupt: 'yydb.corrupt',
    /** Schema 文档与已持久化内容冲突 */
    schemaConflict: 'yydb.schema.conflict',
    /** Schema 文档不合法 */
    schemaInvalid: 'yydb.schema.invalid',
    /** 宿主不支持该能力 */
    unsupported: 'yydb.unsupported',
    /** 会话或数据库已关闭 */
    sessionClosed: 'yydb.session.closed',
    /** 单写者租约不可用（例如缺少 Web Locks） */
    leaseUnavailable: 'yydb.lease.unavailable',
    /** 存储配额不足 */
    quotaExceeded: 'yydb.quota.exceeded',
} as const;

/** 相同参数重试即可成功的错误码 */
const RETRYABLE_CODES: ReadonlySet<string> = new Set([LightDbErrorCodes.leaseUnavailable]);

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
        this.name = 'LightDbError';
        this.code = code;
        this.operation = options.operation ?? 'unknown';
        this.retryable = RETRYABLE_CODES.has(code);
        Object.setPrototypeOf(this, LightDbError.prototype);
    }
}
