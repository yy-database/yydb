/**
 * @yydb/yydb-client 公共错误面。
 *
 * 与 Node 嵌入式库 `@yydb/yydb` 共享同一错误合同：稳定 `code` 字符串、
 * `operation`、`retryable`、`params`。**分支依据是 `code` 不是 `message`。**
 *
 * 引擎侧错误 code 字符串来自 Rust 内核（经 wire 协议传入），这里不维护
 * 引擎 code 常量表——Rust 新增变体时 host 直接拿到字符串即可。
 *
 * Client 本地错误（`yydb.client.*`）只在本包 mint，不流入 Rust。
 *
 * 本包与 `@yydb/yydb` **互不依赖**——错误合同在各自边界独立持有。
 */

/**
 * Client 侧专属错误 code（永远不来自 Rust）。
 */
export const YydbClientErrorCodes = {
    /** serve() endpoint 为空或无效。 */
    clientInvalidEndpoint: 'yydb.client.invalid_endpoint',
    /** connect() socket / transport 层失败。 */
    clientTransportFailure: 'yydb.client.transport_failure',
    /** Server 返回的 request id 与请求不匹配。 */
    clientRequestIdMismatch: 'yydb.client.request_id_mismatch',
    /** Server 返回了未请求的 msg_type。 */
    clientUnexpectedResponse: 'yydb.client.unexpected_response',
    /** 无法解析 wire Error frame body。 */
    clientDecodeFailure: 'yydb.client.decode_failure',
    /** YYDB session 已关闭。 */
    sessionClosed: 'yydb.session.closed',
} as const;

/**
 * 所有已知 client error code 的联合（引擎 code 作为字符串直接使用）。
 */
export const YydbErrorCodes = {
    ...YydbClientErrorCodes,
} as const;

/** 稳定错误 code 字符串。引擎 code 来自 Rust，client code 来自本包常量。 */
export type YydbErrorCode = string;

/** {@link YydbError} 构造参数。 */
export interface YydbErrorOptions {
    /** 失败操作（`schemaGet`、`kvPut`、`query` …）。 */
    operation?: string;
    /**
     * 相同参数是否可重试。
     *
     * 缺省为 `false`（保守）。client 侧 mint 的可重试错误（如
     * `yydb.client.transport_failure`）**必须**显式设置。引擎侧经 wire 传入
     * 的错误携带 Rust 权威 retryable 值，解析时覆盖默认。
     */
    retryable?: boolean;
    /** 结构化参数，供 catalog 模板化渲染使用。 */
    params?: Readonly<Record<string, string>>;
    /** 被本错误包装的底层失败。 */
    cause?: unknown;
}

/** 带稳定机器可读 `code` 的 YYDB 错误。 */
export class YydbError extends Error {
    /** 稳定 code —— 引擎侧来自 Rust，或 `yydb.client.*`。 */
    readonly code: string;
    /** 失败操作。 */
    readonly operation: string;
    /** 是否可重试。 */
    readonly retryable: boolean;
    /** 结构化参数（存在时）。 */
    readonly params?: Readonly<Record<string, string>>;

    constructor(code: YydbErrorCode, message: string, options: YydbErrorOptions = {}) {
        super(message, { cause: options.cause });
        this.name = 'YydbError';
        this.code = code;
        this.operation = options.operation ?? 'unknown';
        this.retryable = options.retryable ?? false;
        if (options.params !== undefined) {
            this.params = options.params;
        }
    }

    /**
     * 从 wire error body（JSON）解析出 {@link YydbError}。格式：
     * `{ code, operation?, retryable?, message, params? }`。
     *
     * 返回解析后的 {@link YydbError}；纯文本输入返回 `null`。
     */
    static fromDiagnosticJson(value: unknown): YydbError | null {
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
        // WASM envelopes nest the diagnostic under `{ ok, error: {...} }`.
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
        const opts: YydbErrorOptions = {};
        if (operation !== undefined) opts.operation = operation;
        if (retryable !== undefined) opts.retryable = retryable;
        if (params !== undefined) opts.params = params;
        return new YydbError(code, message, opts);
    }
}

/** 让调用方无需 `instanceof` import 即可收窄。 */
export function isYydbError(value: unknown): value is YydbError {
    return value instanceof YydbError;
}
