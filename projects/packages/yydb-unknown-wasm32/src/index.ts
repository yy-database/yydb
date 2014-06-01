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
    engineVersion: string;
    error?: string;
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
    query(source: string): string;
    execute(source: string): string;
    get(key: string): string;
    put(key: string, value: Uint8Array): string;
    getSchema(): string;
    info(): string;
    setMicroHostInvoker(invoker: (payload: MicroHostInvokePayload) => unknown): void;
    registerMicro(body: Uint8Array): string;
    callScalar(body: Uint8Array): string;
    close(): void;
    readonly path?: string;
};

const glueApi = glue as {
    yydbVersion?: () => string;
    checkSchema?: (source: string) => CheckSchemaResult;
    introspectSchema?: (source: string) => string;
    queryMemory?: (source: string) => string;
    MemorySession?: new () => WasmSession;
    PersistentSession?: new (path: string) => WasmSession;
};

let ready = false;

function assertReady(): void {
    if (!ready) {
        throw new Error('@yydb/yydb-unknown-wasm32: call initWasm() before semantic core methods');
    }
}

async function resolveDefaultWasmBytes(): Promise<ArrayBuffer | Uint8Array | URL> {
    const wasmUrl = new URL('../lib/yydb_wasm_bg.wasm', import.meta.url);
    if (typeof process !== 'undefined' && process.versions?.node) {
        const { readFileSync } = await import('node:fs');
        const { fileURLToPath } = await import('node:url');
        return readFileSync(fileURLToPath(wasmUrl));
    }
    return wasmUrl;
}

async function normalizeInitInput(input: WasmInitInput | undefined): Promise<WasmInitInput | ArrayBuffer | Uint8Array> {
    if (input === undefined) {
        return resolveDefaultWasmBytes();
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
    const moduleOrPath = await normalizeInitInput(options.module);
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

/** Execute a VOS query on a fresh in-memory database. Returns JSON `{ ok, rows, error }`. */
export function queryMemory(source: string): string {
    assertReady();
    if (!glueApi.queryMemory) {
        throw new Error('@yydb/yydb-unknown-wasm32: queryMemory export missing, rebuild wasm artifacts');
    }
    return glueApi.queryMemory(source);
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
