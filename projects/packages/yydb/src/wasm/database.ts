import {
    checkSchema,
    createMemorySession,
    initWasm,
    openPersistentSession,
    yydbVersion,
    type CheckSchemaResult,
    type InitWasmOptions,
    type WasmSession,
} from './binding.js';
import {
    parseWasmKvGetResult,
    parseWasmRowsResult,
    parseWasmSchemaResult,
    parseWasmUnitResult,
    type SchemaVersion,
} from '../shared/wasm-envelope.js';

export type WasmOpenOptions = InitWasmOptions;

type SessionKind = 'memory' | 'persistent';

/**
 * Browser-facing YYDB handle backed by an in-process wasm session.
 * Use `open` for OPFS-backed paths and `openInMemory` for ephemeral state.
 */
export class Database {
    readonly path: string;
    private session: WasmSession | null;
    private readonly kind: SessionKind;

    private constructor(kind: SessionKind, path: string, session: WasmSession) {
        this.kind = kind;
        this.path = path;
        this.session = session;
    }

    /** Open an OPFS-backed database at `path` (for example `app.yydb`). */
    static async open(path: string, options: WasmOpenOptions = {}): Promise<Database> {
        await initWasm(options);
        const session = openPersistentSession(path);
        return new Database('persistent', session.path ?? path, session);
    }

    /** Open a stateful in-memory database session. */
    static async openInMemory(options: WasmOpenOptions = {}): Promise<Database> {
        await initWasm(options);
        const session = createMemorySession();
        return new Database('memory', ':memory:', session);
    }

    /** Library version from the wasm binding. */
    version(): string {
        return yydbVersion();
    }

    /** Parse and validate a VOS schema document without opening a session. */
    checkSchema(source: string): CheckSchemaResult {
        return checkSchema(source);
    }

    /** Persist schema document. `version` is accepted for Node API parity; wasm uses the document body. */
    async ensureSchema(_version: number, document: string): Promise<void> {
        parseWasmUnitResult(this.requireSession().ensureSchema(document));
    }

    async getSchema(): Promise<SchemaVersion | null> {
        return parseWasmSchemaResult(this.requireSession().getSchema());
    }

    async get(key: string): Promise<Uint8Array | null> {
        return parseWasmKvGetResult(this.requireSession().get(key));
    }

    async put(key: string, value: Uint8Array | string): Promise<void> {
        const bytes = typeof value === 'string' ? new TextEncoder().encode(value) : value;
        parseWasmUnitResult(this.requireSession().put(key, bytes));
    }

    /** VOS read pipeline (for example `User.filter(x => x.active).collect()`). */
    query(source: string): Record<string, unknown>[] {
        return parseWasmRowsResult(this.requireSession().query(source)).rows;
    }

    /** Unit-valued VOS write programs (for example `User { … }.insert()`). */
    execute(source: string): void {
        parseWasmUnitResult(this.requireSession().execute(source));
    }

    /** Whether this handle owns a persistent OPFS session. */
    isPersistent(): boolean {
        return this.kind === 'persistent';
    }

    close(): void {
        this.session?.close();
        this.session = null;
    }

    private requireSession(): WasmSession {
        if (!this.session) {
            throw new Error('@yydb/yydb/wasm: database session is closed');
        }
        return this.session;
    }
}
