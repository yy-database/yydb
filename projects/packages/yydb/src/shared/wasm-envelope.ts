import { YydbError, YydbErrorCodes } from '@yydb/yydb-client';

/** Parsed `{ ok, rows, error, code }` envelope from wasm unit-valued calls. */
export type WasmRowsResult = {
    ok: boolean;
    rows: Record<string, unknown>[];
    error: string | null;
    code: string | null;
};

/** Parsed `{ ok, schema, error, code }` envelope from wasm `getSchema`. */
export type WasmSchemaResult = {
    ok: boolean;
    schema: { version: number; document: string } | null;
    error: string | null;
    code: string | null;
};

/** Parsed `{ ok, value, error, code }` envelope from wasm `get`. */
export type WasmKvGetResult = {
    ok: boolean;
    value: number[] | null;
    error: string | null;
    code: string | null;
};

/** Parsed `{ ok, value, error, code }` envelope from wasm `callScalar`. */
export type WasmScalarResult = {
    ok: boolean;
    value: string | number | boolean | null;
    error: string | null;
    code: string | null;
};

export type WireScalarValue = string | number | boolean | null;

export type SchemaVersion = {
    version: number;
    document: string;
};

type EnvelopeErrorFields = {
    error: string | null;
    code: string | null;
};

function parseJson<T>(payload: string, operation: string): T {
    try {
        return JSON.parse(payload) as T;
    } catch (cause) {
        throw new YydbError(YydbErrorCodes.clientDecodeFailure, `${operation}: invalid JSON envelope`, { operation, cause });
    }
}

/**
 * Build the error for a failed wasm envelope. The Rust core always sends a
 * stable `code`; a missing one means the wasm artifact is stale, which is a
 * decode failure rather than an engine failure.
 */
function envelopeError(envelope: EnvelopeErrorFields, operation: string): YydbError {
    const code = envelope.code ?? YydbErrorCodes.clientDecodeFailure;
    const message = envelope.error ?? `${operation}: wasm envelope carried no message`;
    return new YydbError(code, message, { operation });
}

export function parseWasmUnitResult(payload: string, operation: string): void {
    const parsed = parseJson<WasmRowsResult>(payload, operation);
    if (!parsed.ok) {
        throw envelopeError(parsed, operation);
    }
}

export function parseWasmSchemaResult(payload: string, operation: string): SchemaVersion | null {
    const parsed = parseJson<WasmSchemaResult>(payload, operation);
    if (!parsed.ok) {
        throw envelopeError(parsed, operation);
    }
    return parsed.schema;
}

export function parseWasmScalarResult(payload: string, operation: string): WireScalarValue {
    const parsed = parseJson<WasmScalarResult>(payload, operation);
    if (!parsed.ok) {
        throw envelopeError(parsed, operation);
    }
    return parsed.value;
}

export function parseWasmKvGetResult(payload: string, operation: string): Uint8Array | null {
    const parsed = parseJson<WasmKvGetResult>(payload, operation);
    if (!parsed.ok) {
        throw envelopeError(parsed, operation);
    }
    if (parsed.value === null) {
        return null;
    }
    return Uint8Array.from(parsed.value);
}
