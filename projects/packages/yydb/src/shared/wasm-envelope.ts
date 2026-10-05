/** Parsed `{ ok, rows, error }` envelope from wasm VOS calls. */
export type WasmRowsResult = {
    ok: boolean;
    rows: Record<string, unknown>[];
    error: string | null;
};

/** Parsed `{ ok, schema, error }` envelope from wasm `getSchema`. */
export type WasmSchemaResult = {
    ok: boolean;
    schema: { version: number; document: string } | null;
    error: string | null;
};

/** Parsed `{ ok, value, error }` envelope from wasm `get`. */
export type WasmKvGetResult = {
    ok: boolean;
    value: number[] | null;
    error: string | null;
};

/** Parsed `{ ok, value, error }` envelope from wasm `callScalar`. */
export type WasmScalarResult = {
    ok: boolean;
    value: string | number | boolean | null;
    error: string | null;
};

export type WireScalarValue = string | number | boolean | null;

export type SchemaVersion = {
    version: number;
    document: string;
};

function parseJson<T>(payload: string, label: string): T {
    try {
        return JSON.parse(payload) as T;
    } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        throw new Error(`${label}: invalid JSON (${message})`);
    }
}

export function parseWasmRowsResult(payload: string): WasmRowsResult {
    const parsed = parseJson<WasmRowsResult>(payload, 'wasm query/execute');
    if (!parsed.ok) {
        throw new Error(parsed.error ?? 'wasm query/execute failed');
    }
    return parsed;
}

export function parseWasmUnitResult(payload: string): void {
    const parsed = parseJson<WasmRowsResult>(payload, 'wasm unit call');
    if (!parsed.ok) {
        throw new Error(parsed.error ?? 'wasm unit call failed');
    }
}

export function parseWasmSchemaResult(payload: string): SchemaVersion | null {
    const parsed = parseJson<WasmSchemaResult>(payload, 'wasm getSchema');
    if (!parsed.ok) {
        throw new Error(parsed.error ?? 'wasm getSchema failed');
    }
    return parsed.schema;
}

export function parseWasmScalarResult(payload: string): WireScalarValue {
    const parsed = parseJson<WasmScalarResult>(payload, 'wasm callScalar');
    if (!parsed.ok) {
        throw new Error(parsed.error ?? 'wasm callScalar failed');
    }
    return parsed.value;
}

export function parseWasmKvGetResult(payload: string): Uint8Array | null {
    const parsed = parseJson<WasmKvGetResult>(payload, 'wasm get');
    if (!parsed.ok) {
        throw new Error(parsed.error ?? 'wasm get failed');
    }
    if (parsed.value === null) {
        return null;
    }
    return Uint8Array.from(parsed.value);
}
