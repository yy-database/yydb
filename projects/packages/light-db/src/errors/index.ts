/**
 * 稳定错误模型
 *
 * 与 `@yydb/yydb` / `@yydb/yydb-client` 共用同一组错误码：宿主只按 `code` 分支，
 * `message` 只用于显示。所有边界表面（N-API reason field、WASM JSON envelope、
 * wire protocol）都承载 `yydb::Error::to_json` 产生的同一 `{ code, operation,
 * retryable, message, params? }` 合同。
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
const RETRYABLE_CODES: ReadonlySet<string> = new Set([
    LightDbErrorCodes.leaseUnavailable,
    // WASM core 还可能返回这些 engine 码，LightDbError 也要认
    'yydb.lease.expired',
    'yydb.protocol',
    'yydb.cas.conflict',
]);

/** 数据库错误选项 */
export interface LightDbErrorOptions {
    /** 失败的操作（`schemaGet`、`kvPut` 等） */
    operation?: string;
    /** 结构化参数（catalog 模板用） */
    params?: Readonly<Record<string, string>>;
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

    /** 结构化参数（存在时） */
    public readonly params?: Readonly<Record<string, string>>;

    constructor(code: string, message: string, options: LightDbErrorOptions = {}) {
        super(message, { cause: options.cause });
        this.name = 'LightDbError';
        this.code = code;
        this.operation = options.operation ?? 'unknown';
        this.retryable = RETRYABLE_CODES.has(code);
        if (options.params !== undefined) {
            this.params = options.params;
        }
        Object.setPrototypeOf(this, LightDbError.prototype);
    }

    /**
     * 解析 N-API / WASM 边界表面塞在 message 里的诊断 JSON。
     *
     * ```ts
     * try {
     *     await db.put('key', value);
     * } catch (err) {
     *     const parsed = LightDbError.fromDiagnosticJson(
     *         err instanceof Error ? err.message : String(err),
     *     );
     *     if (parsed) {
     *         // branch on parsed.code / retryable / operation
     *     }
     * }
     * ```
     */
    static fromDiagnosticJson(value: unknown): LightDbError | null {
        let obj: Record<string, unknown>;
        if (typeof value === 'string') {
            try {
                const parsed = JSON.parse(value);
                if (!parsed || typeof parsed !== 'object') return null;
                obj = parsed as Record<string, unknown>;
            } catch {
                return null;
            }
        } else if (value && typeof value === 'object') {
            obj = value as Record<string, unknown>;
        } else {
            return null;
        }
        // WASM envelope 会是 { ok: false, error: { code, operation, ... } }，
        // 也可能直接 { code, operation, ... }。
        let diag = obj;
        if (typeof obj['ok'] === 'boolean' && !obj['ok'] && obj['error'] && typeof obj['error'] === 'object') {
            diag = obj['error'] as Record<string, unknown>;
        }
        const code = typeof diag['code'] === 'string' ? diag['code'] : null;
        const message = typeof diag['message'] === 'string' ? diag['message'] : '';
        if (!code) return null;
        const operation = typeof diag['operation'] === 'string' ? diag['operation'] : undefined;
        const retryable = typeof diag['retryable'] === 'boolean' ? diag['retryable'] : undefined;
        const params =
            diag['params'] && typeof diag['params'] === 'object'
                ? (diag['params'] as Record<string, string>)
                : undefined;
        const options: LightDbErrorOptions = {};
        if (operation !== undefined) options.operation = operation;
        if (params !== undefined) options.params = params;
        const err = new LightDbError(code, message, options);
        if (typeof retryable === 'boolean') {
            (err as LightDbError & { retryable: boolean }).retryable = retryable;
        }
        return err;
    }
}
