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

/**
 * OPFS files the host opened on behalf of the WASM core.
 *
 * Each value is a `FileSystemSyncAccessHandle` obtained inside a worker. The
 * core cannot create one itself: opfs is promise based and sync access handles
 * are worker only, so a durable session always starts from handles the host
 * already holds.
 */
export type PersistentHandles = {
    /** `FileSystemSyncAccessHandle` for `<path>`. */
    main: unknown;
    /** `FileSystemSyncAccessHandle` for `<path>-wal`. */
    wal?: unknown;
    /** `FileSystemSyncAccessHandle` for `<path>-shm`. */
    shm?: unknown;
    /**
     * Synchronous facade the host builds from `FileSystemDirectoryHandle.values()`
     * and `removeEntry()`. Required for `.yydx` volumes, which publish blobs as
     * sibling files; without it the core refuses with `opfs.blob_directory`.
     */
    directory?: {
        list(): string[];
        remove(name: string): void;
    };
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

/** Content-addressed reference to the chunk manifest object behind a `file` value. */
export type FileManifestReference = { readonly hash: string; readonly size: number };

/** Tagged JSON form of an immutable `file` value: identity digest plus manifest reference. */
export type FileScalar = {
    readonly $yydb: 'file';
    readonly digest: string;
    readonly octets: number;
    readonly name?: string;
    readonly mediaType?: string;
    readonly manifest: FileManifestReference;
};

/** Every exact scalar crossing the WASM JSON boundary uses this tagged form. */
export type ScalarValue = DateScalar | TimeScalar | TimestampScalar | DecimalScalar | FileScalar;

/** Declared execution type of a catalog field. */
export type SchemaFieldType =
    | { readonly kind: 'null' | 'bool' | 'i64' | 'text' | 'bytes' | 'file' | 'date' }
    | { readonly kind: 'time'; readonly unit: 'nanosecond'; readonly timezone: 'none' }
    | { readonly kind: 'timestamp'; readonly unit: 'microsecond'; readonly timezone: 'utc' }
    | { readonly kind: 'decimal'; readonly precision: number; readonly scale: number }
    | { readonly kind: 'vector'; readonly dimension: number }
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

/** A wire frame decoded by the Rust core. */
export type DecodedFrame = {
    readonly msgType: number;
    readonly flags: number;
    readonly requestId: number;
    readonly body: Uint8Array;
    readonly product: string;
};

/**
 * YY wire protocol codec, owned by the Rust core (`database.protocol.yy`).
 * Every frame layout, header bound and body encoding rule lives upstream;
 * this surface only bridges it onto JS types. Requires {@link initWasm}.
 */
export type WireApi = {
    version(): string;
    versionNext(): string;
    magic(product: string): Uint8Array;
    headerLen(): number;
    maxBodyLen(): number;
    responseType(code: number): number | null;
    frameEncode(msgType: number, flags: number, requestId: number, body: Uint8Array, product: string): Uint8Array;
    frameDecode(bytes: Uint8Array): DecodedFrame;
    encodeSchemaEnsure(version: number, document: string): Uint8Array;
    decodeSchemaEnsure(body: Uint8Array): string;
    encodeKvGet(key: string): Uint8Array;
    decodeKvGet(body: Uint8Array): string;
    encodeKvPut(key: string, value: Uint8Array): Uint8Array;
    decodeKvPut(body: Uint8Array): string;
    encodeAuthenticate(token: string): Uint8Array;
    decodeAuthenticateOk(body: Uint8Array): string;
    decodeSchemaGetOk(body: Uint8Array): string | null;
    decodeKvGetOk(body: Uint8Array): Uint8Array | null;
    encodeScalarCall(requestJson: string): Uint8Array;
    decodeScalarCallOk(body: Uint8Array): string;
    encodeMicroRegister(payloadJson: string): Uint8Array;
    decodeMicroRegister(body: Uint8Array): string;
    decodeMicroHostInvoke(body: Uint8Array): string;
    encodeMicroHostInvokeOk(valueJson: string): Uint8Array;
    encodeErrorBody(diagnosticJson: string): Uint8Array;
    decodeErrorBody(body: Uint8Array): string;
};

/** Stateful wasm session (memory or OPFS-backed). */
export type WasmSession = {
    ensureSchema(document: string): string;
    get(key: string): string;
    put(key: string, value: Uint8Array): string;
    delete (key: string): string;
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
    /** Roll back the current transaction. Returns JSON `{ ok, error? }`. */
    rollback(): string;
    /** Create a named savepoint inside the open transaction. Returns JSON `{ ok, error? }`. */
    savepoint(name: string): string;
    /** Roll back to a savepoint. Returns JSON `{ ok, error? }`. */
    rollbackTo(name: string): string;
    /** Release a savepoint. Returns JSON `{ ok, error? }`. */
    release(name: string): string;
    /** Close the session; a failed final durable publication throws. */
    close(): void;
    readonly path?: string;
};

/** Durable session: adds the generation of the last main-file publication. */
export type PersistentWasmSession = WasmSession & {
    readonly generation: number;
};

const glueApi = glue as {
    yydbVersion?: () => string;
    checkSchema?: (source: string) => CheckSchemaResult;
    introspectSchema?: (source: string) => string;
    MemorySession?: (new () => WasmSession) & {
        restoreFromSnapshot?: (bytes: Uint8Array) => WasmSession;
    };
    PersistentSession?: (new (path: string) => WasmSession) & {
        openWithHandles?: (path: string, handles: PersistentHandles) => PersistentWasmSession;
        restoreBackup?: (
            path: string,
            bytes: Uint8Array,
            handles: PersistentHandles,
        ) => PersistentWasmSession;
    };
    wireVersion?: () => string;
    wireVersionNext?: () => string;
    wireMagic?: (product: string) => Uint8Array;
    wireHeaderLen?: () => number;
    wireMaxBodyLen?: () => number;
    responseType?: (code: number) => number | undefined;
    frameEncode?: (msgType: number, flags: number, requestId: number, body: Uint8Array, product: string) => Uint8Array;
    frameDecode?: (bytes: Uint8Array) => DecodedFrame;
    encodeSchemaEnsure?: (version: number, document: string) => Uint8Array;
    decodeSchemaEnsure?: (body: Uint8Array) => string;
    encodeKvGet?: (key: string) => Uint8Array;
    decodeKvGet?: (body: Uint8Array) => string;
    encodeKvPut?: (key: string, value: Uint8Array) => Uint8Array;
    decodeKvPut?: (body: Uint8Array) => string;
    encodeAuthenticate?: (token: string) => Uint8Array;
    decodeAuthenticateOk?: (body: Uint8Array) => string;
    decodeSchemaGetOk?: (body: Uint8Array) => string | undefined;
    decodeKvGetOk?: (body: Uint8Array) => Uint8Array | undefined;
    encodeScalarCall?: (requestJson: string) => Uint8Array;
    decodeScalarCallOk?: (body: Uint8Array) => string;
    encodeMicroRegister?: (payloadJson: string) => Uint8Array;
    decodeMicroRegister?: (body: Uint8Array) => string;
    decodeMicroHostInvoke?: (body: Uint8Array) => string;
    encodeMicroHostInvokeOk?: (valueJson: string) => Uint8Array;
    encodeErrorBody?: (diagnosticJson: string) => Uint8Array;
    decodeErrorBody?: (body: Uint8Array) => string;
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

/**
 * Open an in-memory session from a main-file snapshot.
 *
 * The snapshot is verified inside the WASM core (page CRC32C plus a doctor
 * pass) before the session takes ownership of it, so a damaged backup throws
 * instead of producing a half-readable database.
 */
export function restoreMemorySession(bytes: Uint8Array): WasmSession {
    assertReady();
    const restore = glueApi.MemorySession?.restoreFromSnapshot;
    if (!restore) {
        throw new Error('@yydb/yydb-unknown-wasm32: MemorySession.restoreFromSnapshot missing, rebuild wasm artifacts');
    }
    return restore(bytes);
}

/**
 * Replace the OPFS database at `path` with a verified snapshot and open it.
 *
 * The snapshot is validated before it is published, so a damaged backup never
 * overwrites a live database. The host must pass the sync access handles it
 * opened for `path`.
 */
export function restorePersistentSession(
    path: string,
    bytes: Uint8Array,
    handles: PersistentHandles,
): PersistentWasmSession {
    assertReady();
    const restore = glueApi.PersistentSession?.restoreBackup;
    if (!restore) {
        throw new Error('@yydb/yydb-unknown-wasm32: PersistentSession.restoreBackup missing, rebuild wasm artifacts');
    }
    return restore(path, bytes, handles);
}

/**
 * Open a stateful OPFS-backed session at `path` over host sync handles.
 *
 * The host opens `<path>`, `<path>-wal` and `<path>-shm` inside a worker,
 * creates a `FileSystemSyncAccessHandle` for each and hands them over.
 */
export function openPersistentSessionWithHandles(path: string, handles: PersistentHandles): PersistentWasmSession {
    assertReady();
    const open = glueApi.PersistentSession?.openWithHandles;
    if (!open) {
        throw new Error('@yydb/yydb-unknown-wasm32: PersistentSession.openWithHandles missing, rebuild wasm artifacts');
    }
    return open(path, handles);
}

/**
 * Open a stateful OPFS-backed session at `path`.
 *
 * A path alone is not enough for a durable browser volume: OPFS access is
 * asynchronous and sync access handles are worker only. This entry therefore
 * throws the structured `yydb.native_capability_missing` error. Use
 * {@link openPersistentSessionWithHandles} with handles opened by the host.
 */
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

function wireExport<T>(name: string, fn: T | undefined): T {
    if (!fn) {
        throw new Error(`@yydb/yydb-unknown-wasm32: ${name} export missing, rebuild wasm artifacts`);
    }
    return fn;
}

function wireGuard<R>(run: () => R): R {
    try {
        return run();
    } catch (error) {
        throw error instanceof Error ? error : new Error(String(error));
    }
}

/**
 * YY wire protocol codec backed by the Rust core.
 *
 * Frame layout, header bounds and every body encoding rule are owned
 * upstream; nothing here re-declares the wire. Call {@link initWasm} first.
 */
export const wire: WireApi = {
    version(): string {
        assertReady();
        return wireGuard(() => wireExport('wireVersion', glueApi.wireVersion)());
    },
    versionNext(): string {
        assertReady();
        return wireGuard(() => wireExport('wireVersionNext', glueApi.wireVersionNext)());
    },
    magic(product: string): Uint8Array {
        assertReady();
        return wireGuard(() => wireExport('wireMagic', glueApi.wireMagic)(product));
    },
    headerLen(): number {
        assertReady();
        return wireGuard(() => wireExport('wireHeaderLen', glueApi.wireHeaderLen)());
    },
    maxBodyLen(): number {
        assertReady();
        return wireGuard(() => wireExport('wireMaxBodyLen', glueApi.wireMaxBodyLen)());
    },
    responseType(code: number): number | null {
        assertReady();
        return wireGuard(() => wireExport('responseType', glueApi.responseType)(code)) ?? null;
    },
    frameEncode(msgType: number, flags: number, requestId: number, body: Uint8Array, product: string): Uint8Array {
        assertReady();
        return wireGuard(() => wireExport('frameEncode', glueApi.frameEncode)(msgType, flags, requestId, body, product));
    },
    frameDecode(bytes: Uint8Array): DecodedFrame {
        assertReady();
        return wireGuard(() => wireExport('frameDecode', glueApi.frameDecode)(bytes));
    },
    encodeSchemaEnsure(version: number, document: string): Uint8Array {
        assertReady();
        return wireGuard(() => wireExport('encodeSchemaEnsure', glueApi.encodeSchemaEnsure)(version, document));
    },
    decodeSchemaEnsure(body: Uint8Array): string {
        assertReady();
        return wireGuard(() => wireExport('decodeSchemaEnsure', glueApi.decodeSchemaEnsure)(body));
    },
    encodeKvGet(key: string): Uint8Array {
        assertReady();
        return wireGuard(() => wireExport('encodeKvGet', glueApi.encodeKvGet)(key));
    },
    decodeKvGet(body: Uint8Array): string {
        assertReady();
        return wireGuard(() => wireExport('decodeKvGet', glueApi.decodeKvGet)(body));
    },
    encodeKvPut(key: string, value: Uint8Array): Uint8Array {
        assertReady();
        return wireGuard(() => wireExport('encodeKvPut', glueApi.encodeKvPut)(key, value));
    },
    decodeKvPut(body: Uint8Array): string {
        assertReady();
        return wireGuard(() => wireExport('decodeKvPut', glueApi.decodeKvPut)(body));
    },
    encodeAuthenticate(token: string): Uint8Array {
        assertReady();
        return wireGuard(() => wireExport('encodeAuthenticate', glueApi.encodeAuthenticate)(token));
    },
    decodeAuthenticateOk(body: Uint8Array): string {
        assertReady();
        return wireGuard(() => wireExport('decodeAuthenticateOk', glueApi.decodeAuthenticateOk)(body));
    },
    decodeSchemaGetOk(body: Uint8Array): string | null {
        assertReady();
        return wireGuard(() => wireExport('decodeSchemaGetOk', glueApi.decodeSchemaGetOk)(body)) ?? null;
    },
    decodeKvGetOk(body: Uint8Array): Uint8Array | null {
        assertReady();
        return wireGuard(() => wireExport('decodeKvGetOk', glueApi.decodeKvGetOk)(body)) ?? null;
    },
    encodeScalarCall(requestJson: string): Uint8Array {
        assertReady();
        return wireGuard(() => wireExport('encodeScalarCall', glueApi.encodeScalarCall)(requestJson));
    },
    decodeScalarCallOk(body: Uint8Array): string {
        assertReady();
        return wireGuard(() => wireExport('decodeScalarCallOk', glueApi.decodeScalarCallOk)(body));
    },
    encodeMicroRegister(payloadJson: string): Uint8Array {
        assertReady();
        return wireGuard(() => wireExport('encodeMicroRegister', glueApi.encodeMicroRegister)(payloadJson));
    },
    decodeMicroRegister(body: Uint8Array): string {
        assertReady();
        return wireGuard(() => wireExport('decodeMicroRegister', glueApi.decodeMicroRegister)(body));
    },
    decodeMicroHostInvoke(body: Uint8Array): string {
        assertReady();
        return wireGuard(() => wireExport('decodeMicroHostInvoke', glueApi.decodeMicroHostInvoke)(body));
    },
    encodeMicroHostInvokeOk(valueJson: string): Uint8Array {
        assertReady();
        return wireGuard(() => wireExport('encodeMicroHostInvokeOk', glueApi.encodeMicroHostInvokeOk)(valueJson));
    },
    encodeErrorBody(diagnosticJson: string): Uint8Array {
        assertReady();
        return wireGuard(() => wireExport('encodeErrorBody', glueApi.encodeErrorBody)(diagnosticJson));
    },
    decodeErrorBody(body: Uint8Array): string {
        assertReady();
        return wireGuard(() => wireExport('decodeErrorBody', glueApi.decodeErrorBody)(body));
    },
};
