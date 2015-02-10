import initGlue, * as glue from '../lib/yydb_wasm.js';

export type WasmInitInput = URL | Request | Response | ArrayBuffer | Uint8Array | WebAssembly.Module;

export type InitWasmOptions = {
    /** When omitted, loads the bundled `lib/yydb_wasm_bg.wasm` asset. */
    module?: WasmInitInput;
};

export type CheckSchemaResult = {
    ok: boolean;
    tableCount: number;
    schemaFingerprint: string;
    error?: string;
};

/** Tagged JSON form of an exact `date` value (days since the Unix epoch). */
export type DateScalar = { readonly $yydb: 'date'; readonly days: number };

/** Tagged JSON form of an exact `time` value (nanoseconds since midnight, no zone). */
export type TimeScalar = { readonly $yydb: 'time'; readonly nanos: number };

/** Tagged JSON form of an exact UTC `timestamp` (microseconds as a decimal string). */
export type TimestampScalar = { readonly $yydb: 'timestamp'; readonly utcMicros: string };

/** Tagged JSON form of an exact fixed-point `decimal` value. */
export type DecimalScalar = {
    readonly $yydb: 'decimal';
    readonly precision: number;
    readonly scale: number;
    readonly unscaled: string;
};

/** Every exact scalar crossing the WASM JSON boundary uses this tagged form. */
export type ScalarValue = DateScalar | TimeScalar | TimestampScalar | DecimalScalar;

/** Declared execution type of a catalog field. */
export type SchemaFieldType =
    | { readonly kind: 'null' | 'bool' | 'i64' | 'text' | 'bytes' | 'file' | 'date' }
    | { readonly kind: 'time'; readonly unit: 'nanosecond'; readonly timezone: 'none' }
    | { readonly kind: 'timestamp'; readonly unit: 'microsecond'; readonly timezone: 'utc' }
    | { readonly kind: 'decimal'; readonly precision: number; readonly scale: number }
    | { readonly kind: 'vector'; readonly dimension: number; readonly metric: 'cosine' | 'euclidean' | 'dot' }
    | { readonly kind: 'record'; readonly schemaId: number; readonly fieldCount: number };

/** One uniqueness rule published by the catalog. */
export type UniqueConstraintDescriptor = {
    readonly name: string;
    readonly columns: string[];
    readonly nullsDistinct: boolean;
};

/** One field in an introspected table. */
export type SchemaFieldDescriptor = {
    readonly name: string;
    readonly fieldId: number;
    readonly virtualField: number;
    readonly type: SchemaFieldType;
};

/** Parsed payload of {@link introspectSchema}. */
export type SchemaIntrospection = {
    readonly ok: boolean;
    readonly schemaFingerprint: string;
    readonly tables: ReadonlyArray<{
        readonly name: string;
        readonly schemaId: number;
        readonly fields: ReadonlyArray<SchemaFieldDescriptor>;
        readonly uniqueConstraints: ReadonlyArray<UniqueConstraintDescriptor>;
    }>;
    readonly error: unknown;
};

export type MicroHostInvokePayload = {
    hostId: number;
    handleVersion: number;
    functionId: string;
    args: readonly unknown[];
};

/** Stateful wasm session (memory or OPFS-backed). */
export type WasmSession = {
    ensureSchema(document: string): string;
    get(key: string): string;
    put(key: string, value: Uint8Array): string;
    delete(key: string): string;
    scanPrefix(prefix: string, after: string | null | undefined, limit: number): string;
    writeBatch(body: Uint8Array): string;
    compareExchange(key: string, expected?: Uint8Array | null, replacement?: Uint8Array | null): string;
    getSchema(): string;
    queryVos(source: string): string;
    executeVosWrite(source: string): string;
    migrateSchema(document: string, renames_json?: string | null): string;
    begin(): string;
    commit(): string;
    rollback(): string;
    checkpoint(): string;
    info(): string;
    setMicroHostInvoker(invoker: (payload: MicroHostInvokePayload) => unknown): void;
    registerMicro(body: Uint8Array): string;
    callScalar(body: Uint8Array): string;
    /** High-level JSON descriptor for host micro registration. */
    registerMicroJson(json: string): string;
    /** High-level JSON args for scalar UDF invocation. */
    callScalarJson(json: string): string;
    /** Read-only consistency probes. Returns JSON `{ ok, report?, error? }`. */
    doctorJson(): string;
    /** Prepare a VOS query plan. Returns JSON `{ ok, statement_id?, ..., error? }`. */
    prepareJson(source: string): string;
    /** Execute a previously prepared statement. Returns JSON `{ ok, rows?, error? }`. */
    queryPreparedJson(json: string): string;
    /** Release a prepared statement's plan. Returns true if present. */
    closeStatement(statementId: string): boolean;
    close(): void;
    readonly path?: string;
};

const glueApi = glue as {
    yydbVersion?: () => string;
    checkSchema?: (source: string) => CheckSchemaResult;
    introspectSchema?: (source: string) => string;
    MemorySession?: new () => WasmSession;
    PersistentSession?: new (path: string) => WasmSession;
};

let ready = false;

function assertReady(): void {
    if (!ready) {
        throw new Error('@yydb/yydb-unknown-wasm32: call initWasm() before semantic core methods');
    }
}

function resolveDefaultWasmUrl(): URL {
    return new URL('../lib/yydb_wasm_bg.wasm', import.meta.url);
}

function normalizeInitInput(input: WasmInitInput | undefined): WasmInitInput | ArrayBuffer | Uint8Array {
    if (input === undefined) {
        return resolveDefaultWasmUrl();
    }
    if (
        input instanceof URL ||
        input instanceof Request ||
        input instanceof Response ||
        input instanceof ArrayBuffer ||
        input instanceof Uint8Array ||
        input instanceof WebAssembly.Module
    ) {
        return input;
    }
    throw new Error(`@yydb/yydb-unknown-wasm32: unsupported WASM init input (${typeof input})`);
}

/** One-time WASM init. Required before semantic core methods. */
export async function initWasm(options: InitWasmOptions = {}): Promise<void> {
    if (ready) {
        return;
    }
    const moduleOrPath = normalizeInitInput(options.module);
    await initGlue({ module_or_path: moduleOrPath });
    ready = true;
}

/** Library version (matches workspace `@yydb/yydb` semver). */
export function yydbVersion(): string {
    assertReady();
    if (!glueApi.yydbVersion) {
        throw new Error('@yydb/yydb-unknown-wasm32: yydbVersion export missing, rebuild wasm artifacts');
    }
    return glueApi.yydbVersion();
}

/** Parse and validate a VOS schema document via the Rust core. */
export function checkSchema(source: string): CheckSchemaResult {
    assertReady();
    if (!glueApi.checkSchema) {
        throw new Error('@yydb/yydb-unknown-wasm32: checkSchema export missing, rebuild wasm artifacts');
    }
    return glueApi.checkSchema(source);
}

/** Read-only schema introspection JSON. */
export function introspectSchema(source: string): string {
    assertReady();
    if (!glueApi.introspectSchema) {
        throw new Error('@yydb/yydb-unknown-wasm32: introspectSchema export missing, rebuild wasm artifacts');
    }
    return glueApi.introspectSchema(source);
}

/** Open a stateful in-memory session. */
export function createMemorySession(): WasmSession {
    assertReady();
    if (!glueApi.MemorySession) {
        throw new Error('@yydb/yydb-unknown-wasm32: MemorySession export missing, rebuild wasm artifacts');
    }
    return new glueApi.MemorySession();
}

/** Open a stateful OPFS-backed session at `path`. */
export function openPersistentSession(path: string): WasmSession {
    assertReady();
    if (!glueApi.PersistentSession) {
        throw new Error('@yydb/yydb-unknown-wasm32: PersistentSession export missing, rebuild wasm artifacts');
    }
    return new glueApi.PersistentSession(path);
}

/** @internal Reset init gate (binding tests only). */
export function resetWasmBindingForTests(): void {
    ready = false;
}
