/**
 * Family error envelope for the formal `Yydb` / `YydbClient` constructors.
 *
 * The envelope unifies *localisation* and *result certainty* only:
 * `product`, `code`, `phase`, `execution` and `retry`. Product-specific
 * detail (engine codes with the `yydb.` prefix, VOS diagnostics) rides on
 * `code` and `cause`; hosts branch on `code`, never on `message`.
 *
 * Native failures arrive as plain `Error`s whose message embeds the
 * diagnostic JSON emitted by the Rust core (`{ code, operation, retryable,
 * message, params? }`). {@link DatabaseError.from} maps those envelopes onto
 * this shape without inventing a second error taxonomy.
 */

import { YydbError } from './errors.js';

/** Lifecycle stage in which the failure happened. */
export type DatabaseErrorPhase =
    | 'configuration'
    | 'open'
    | 'connect'
    | 'execute'
    | 'commit'
    | 'close';

/** Whether the operation ever reached the executor and whether its reply is known. */
export type DatabaseErrorExecution = 'not-sent' | 'unknown' | 'replied';

/**
 * Retry guidance for the caller. `safe` means the same call may be repeated
 * with the same arguments; `application-decision` means the outcome is not
 * locally decidable (typically a commit whose reply was lost).
 */
export type DatabaseErrorRetry = 'never' | 'safe' | 'application-decision';

/** Options accepted by the {@link DatabaseError} constructor. */
export interface DatabaseErrorInit {
    readonly phase: DatabaseErrorPhase;
    readonly execution: DatabaseErrorExecution;
    /** Explicit override; otherwise derived from `execution` and `retryable`. */
    readonly retry?: DatabaseErrorRetry;
    readonly cause?: unknown;
}

/**
 * YYDB product error carrying the family envelope. `product` is fixed to
 * `'yydb'`; codes keep the native `yydb.` prefix so a mapped engine code and
 * a host-minted code share one namespace.
 */
export class DatabaseError extends Error {
    readonly product = 'yydb' as const;
    readonly code: string;
    readonly phase: DatabaseErrorPhase;
    readonly execution: DatabaseErrorExecution;
    readonly retry: DatabaseErrorRetry;

    constructor(code: string, message: string, init: DatabaseErrorInit) {
        super(message, init.cause === undefined ? undefined : { cause: init.cause });
        this.name = 'DatabaseError';
        this.code = code;
        this.phase = init.phase;
        this.execution = init.execution;
        this.retry = init.retry ?? deriveRetry(init.execution, false);
    }

    /**
     * Map any thrown value onto the envelope.
     *
     * - `DatabaseError` passes through untouched (already localised).
     * - `YydbError` (engine or client code) keeps its `code` and Rust-side
     *   `retryable` flag.
     * - A plain `Error` whose message is a native diagnostic JSON envelope is
     *   decoded through `YydbError.fromDiagnosticJson` first, so every
     *   `yydb.`-prefixed code crossing N-API or the wire lands here intact.
     * - Anything else surfaces as `yydb.unknown` with the original as `cause`.
     */
    static from(cause: unknown, init: DatabaseErrorInit): DatabaseError {
        if (cause instanceof DatabaseError) {
            return cause;
        }
        const mapped = cause instanceof YydbError
            ? cause
            // Object-shaped diagnostics first (engine `YydbError`s and the
            // structurally identical errors from `@yydb/yydb-client` cross a
            // class boundary and must not degrade to `yydb.unknown`), then the
            // JSON-in-message form N-API and the WASM surface embed in `Error`.
            : YydbError.fromDiagnosticJson(cause)
                ?? YydbError.fromDiagnosticJson(cause instanceof Error ? cause.message : null);
        if (!mapped) {
            return new DatabaseError('yydb.unknown', cause instanceof Error ? cause.message : String(cause), {
                ...init,
                retry: init.retry ?? deriveRetry(init.execution, false),
                cause,
            });
        }
        return new DatabaseError(mapped.code, mapped.message, {
            ...init,
            retry: init.retry ?? deriveRetry(init.execution, mapped.retryable),
            cause: mapped.cause ?? cause,
        });
    }
}

function deriveRetry(execution: DatabaseErrorExecution, retryable: boolean): DatabaseErrorRetry {
    if (execution === 'unknown') {
        return 'application-decision';
    }
    return retryable ? 'safe' : 'never';
}

/** Narrowing helper so callers can branch without `instanceof` imports. */
export function isDatabaseError(value: unknown): value is DatabaseError {
    return value instanceof DatabaseError;
}
