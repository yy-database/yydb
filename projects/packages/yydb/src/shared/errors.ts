/**
 * Host-facing YYDB error surface.
 *
 * Every public failure carries a stable `code`: engine codes come from the
 * Rust core (`yydb.schema.invalid`, …) over N-API or the wire; client codes
 * are minted locally with the `yydb.client.` prefix. **Branch on `code`** —
 * `message` is display text only and must never be used for retry decisions.
 *
 * ## Where codes live
 *
 * Engine codes live in Rust (`yydb_types::Error` variants → DiagnosticEnvelope).
 * The TS side no longer keeps a hardcoded engine-code table — when Rust gains a
 * new variant, hosts pick up the new code string via {@link fromDiagnosticJson}
 * without any TS change. Client codes (`yydb.client.*`) are minted only on the
 * host side and remain available as constants below.
 */

/**
 * Client-side error codes that never originate from Rust.
 *
 * Engine codes (`yydb.schema.*`, `yydb.udf.*`, `yydb.io`, …) are emitted by
 * Rust and should be matched via string literals or from the diagnostic JSON
 * envelope — they are intentionally not duplicated here.
 */
export const YydbClientErrorCodes = {
    /** serve() endpoint is empty or invalid. */
    clientInvalidEndpoint: 'yydb.client.invalid_endpoint',
    /** connect() socket / transport layer failure. */
    clientTransportFailure: 'yydb.client.transport_failure',
    /** Server replied with a request id we did not send. */
    clientRequestIdMismatch: 'yydb.client.request_id_mismatch',
    /** Server replied with a msg_type we did not request. */
    clientUnexpectedResponse: 'yydb.client.unexpected_response',
    /** Could not parse an Error frame body from the wire. */
    clientDecodeFailure: 'yydb.client.decode_failure',
    /** The YYDB session is already closed. */
    sessionClosed: 'yydb.session.closed',
} as const;

/**
 * @deprecated Engine codes moved to Rust — use string literals or match on
 * {@link YydbError.code} returned from {@link fromDiagnosticJson}. This table
 * exists only for transition; new engine error variants will **not** be added here.
 */
export const YydbErrorCodes = {
    // Engine codes (deprecated)
    io: 'yydb.io',
    corrupt: 'yydb.corrupt',
    schemaConflict: 'yydb.schema.conflict',
    schemaInvalid: 'yydb.schema.invalid',
    udfNotFound: 'yydb.udf.not_found',
    udfArity: 'yydb.udf.arity',
    udfFailed: 'yydb.udf.failed',
    udfVersionMismatch: 'yydb.udf.version_mismatch',
    objectNotFound: 'yydb.object.not_found',
    objectCorrupt: 'yydb.object.corrupt',
    unsupported: 'yydb.unsupported',
    protocol: 'yydb.protocol',
    casConflict: 'yydb.cas.conflict',
    leaseUnavailable: 'yydb.lease.unavailable',
    fencingMismatch: 'yydb.fencing.mismatch',
    leaseExpired: 'yydb.lease.expired',
    quotaExceeded: 'yydb.quota.exceeded',
    // Client codes (still canonical for client-local errors)
    ...YydbClientErrorCodes,
} as const;

/** Any stable YYDB error code string — engine, client, or future variant. */
export type YydbErrorCode = string;

/** Options carried by a {@link YydbError}. */
export interface YydbErrorOptions {
    /** Operation that failed (`schemaGet`, `kvPut`, `query`, …). */
    operation?: string;
    /**
     * Whether the caller may retry with the same arguments.
     *
     * When omitted, constructor defaults to `false` (conservative). Callers
     * that mint their own errors (especially client-local retryable ones like
     * `yydb.client.transport_failure`) **must** set this explicitly. Engine
     * errors coming through {@link fromDiagnosticJson} always carry the Rust-
     * authoritative `retryable` value and override the default.
     */
    retryable?: boolean;
    /** Structured parameters for catalog templating (when present). */
    params?: Readonly<Record<string, string>>;
    /** Underlying failure when this error wraps one. */
    cause?: unknown;
}

/** YYDB failure with a stable machine-readable {@link YydbError.code}. */
export class YydbError extends Error {
    /** Stable code — engine code (from Rust) or `yydb.client.*`. */
    readonly code: string;
    /** Operation that failed. */
    readonly operation: string;
    /** Whether the same call may be retried. */
    readonly retryable: boolean;
    /** Structured parameters for catalog templating (when present). */
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
     * Parse the diagnostic JSON that N-API and WASM surfaces embed in their
     * `Error.message`. Every boundary surface (N-API reason field, WASM JSON
     * envelope, wire protocol) carries the same `{ code, operation, retryable,
     * message, params? }` contract produced by `yydb::Error::to_json`.
     *
     * Returns the parsed {@link YydbError} if `value` is a diagnostic object,
     * or `null` when the input is plain text. Hosts typically use this as a
     * fallback in catch blocks:
     *
     * ```ts
     * try {
     *     await db.put('key', value);
     * } catch (err) {
     *     const parsed = YydbError.fromDiagnosticJson(
     *         err instanceof Error ? err.message : String(err),
     *     );
     *     if (parsed) {
     *         // branch on parsed.code / retryable / operation
     *     }
     * }
     * ```
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

/** Narrowing helper so callers can branch without `instanceof` imports. */
export function isYydbError(value: unknown): value is YydbError {
    return value instanceof YydbError;
}
