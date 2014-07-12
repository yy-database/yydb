/**
 * Host-facing YYDB error surface.
 *
 * Every public failure carries a stable `code`: engine codes come from the
 * Rust core (`yydb.schema.invalid`, …) over the wire, client codes are minted
 * locally with the `yydb.client.` prefix. Branch on `code` — `message` is
 * display text only.
 */

/** Stable codes known to the TypeScript hosts. */
export const YydbErrorCodes = {
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
    sessionClosed: 'yydb.session.closed',
    clientInvalidEndpoint: 'yydb.client.invalid_endpoint',
    clientTransportFailure: 'yydb.client.transport_failure',
    clientRequestIdMismatch: 'yydb.client.request_id_mismatch',
    clientUnexpectedResponse: 'yydb.client.unexpected_response',
    clientDecodeFailure: 'yydb.client.decode_failure',
} as const;

/** A known code, or any future code the engine introduces. */
export type YydbErrorCode = (typeof YydbErrorCodes)[keyof typeof YydbErrorCodes] | (string & {});

/**
 * Codes where retrying the same operation can legitimately succeed: a socket or
 * lease hiccup, or a compare-and-set that lost a benign race. Every other code
 * needs the caller to change input, schema, or wiring first.
 */
const RETRYABLE_CODES: ReadonlySet<string> = new Set([
    YydbErrorCodes.clientTransportFailure,
    YydbErrorCodes.casConflict,
    YydbErrorCodes.leaseUnavailable,
]);

/** Whether `code` may be retried with the same arguments. */
export function isRetryableYydbCode(code: string): boolean {
    return RETRYABLE_CODES.has(code);
}

/** Options carried by a {@link YydbError}. */
export interface YydbErrorOptions {
    /** Operation that failed (`schemaGet`, `kvPut`, `query`, …). */
    operation?: string;
    /** Underlying failure when this error wraps one. */
    cause?: unknown;
}

/** YYDB failure with a stable machine-readable {@link YydbError.code}. */
export class YydbError extends Error {
    /** Stable code — engine code or `yydb.client.*`. */
    readonly code: string;
    /** Operation that failed. */
    readonly operation: string;
    /** Whether the same call may be retried. */
    readonly retryable: boolean;

    constructor(code: YydbErrorCode, message: string, options: YydbErrorOptions = {}) {
        super(message, { cause: options.cause });
        this.name = 'YydbError';
        this.code = code;
        this.operation = options.operation ?? 'unknown';
        this.retryable = isRetryableYydbCode(code);
    }
}

/** Narrowing helper so callers can branch without `instanceof` imports. */
export function isYydbError(value: unknown): value is YydbError {
    return value instanceof YydbError;
}
