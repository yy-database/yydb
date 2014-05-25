import { checkSchema, initWasm, yydbVersion, type CheckSchemaResult, type InitWasmOptions } from './binding.js';

export type WasmOpenOptions = InitWasmOptions;

/**
 * Browser-facing YYDB handle backed by the in-process wasm core.
 * Persistent OPFS sessions will land on this class next.
 */
export class Database {
    private constructor() {}

    /** One-time wasm init required before semantic methods. */
    static async init(options: WasmOpenOptions = {}): Promise<Database> {
        await initWasm(options);
        return new Database();
    }

    /** In-memory wasm entry (alias for `init` until OPFS `open` ships). */
    static async openInMemory(options: WasmOpenOptions = {}): Promise<Database> {
        return Database.init(options);
    }

    /** Library version from the wasm binding. */
    version(): string {
        return yydbVersion();
    }

    /** Parse and validate a VOS schema document via the Rust core. */
    checkSchema(source: string): CheckSchemaResult {
        return checkSchema(source);
    }
}
