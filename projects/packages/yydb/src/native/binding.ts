import { createRequire } from 'node:module';
import fs from 'node:fs';
import path from 'node:path';
import { platformKey } from './resolve.js';

/** Schema version returned by N-API Database.schema_version / Client.schema_version. */
export interface NativeSchemaVersion {
    version: number;
    document: string;
}

/** One open `.yydb` handle exposed by the native binding. */
export interface NativeDatabase {
    path(): string | null;
    walPath(): string | null;
    shmPath(): string | null;
    schemaVersion(): number | null;
    schemaDocument(): string | null;
    ensureSchema(document: string): void;
    recordCount(): number;
    journalMode(): string;
    setJournalMode(mode: string): void;
    checkpoint(): void;
    walFrameCount(): number;
    get(key: string): Uint8Array | null;
    put(key: string, value: Uint8Array): void;
    /** Delete one record by key. */
    delete(key: string): void;
    /** Scan keys with a prefix; returns JSON `[{ key, value }, ...]`. */
    scanPrefix(prefix: string, after: string | null, limit: number): string;
    /** Atomically write or delete many keys; body is JSON `[{ key, value|null }, ...]`. */
    writeBatch(changesJson: string): void;
    /** Compare-and-swap; returns true on success. */
    compareExchange(
        key: string,
        expected: Uint8Array | null,
        replacement: Uint8Array | null,
    ): boolean;
    /** Begin an explicit transaction. */
    begin(): void;
    /** Commit the current transaction. */
    commit(): void;
    /** Roll back the current transaction. */
    rollback(): void;
    /** Execute a VOS read pipeline; returns JSON rows. */
    queryVos(source: string): string;
    /** Execute a VOS write program. */
    executeVosWrite(source: string): void;
    /** Publish a new schema version, optionally with rename map. */
    migrateSchema(document: string, renamesJson?: string | null): void;
    registerMicro(
        name: string,
        version: number,
        args: string[],
        returns: string,
        hostId: number,
        functionId: string,
        handleVersion: number,
        fingerprint: Uint8Array,
    ): void;
    callScalar(name: string, version: number, argsJson: string): string;
}

/** One remote `yydb serve` handle exposed by the native binding. */
export interface NativeClient {
    endpoint(): string;
    serverVersion(): string;
    info(): string;
    schemaVersion(): number | null;
    schemaDocument(): string | null;
    ensureSchema(document: string): void;
    get(key: string): Uint8Array | null;
    put(key: string, value: Uint8Array): void;
    registerMicro(
        name: string,
        version: number,
        args: string[],
        returns: string,
        hostId: number,
        functionId: string,
        handleVersion: number,
        fingerprint: Uint8Array,
    ): void;
    callScalar(name: string, version: number, argsJson: string): string;
    close(): void;
}

/** Atomic operations exported by the Node-API binding. */
export interface YydbNative {
    version(): string;
    serve(dbPath: string, bind: string, insecureBind?: boolean): void;
    Database: { open(dbPath: string): NativeDatabase };
    Client: { connect(endpoint: string): NativeClient };
}

function loadModule(specifier: string): YydbNative {
    const require = createRequire(import.meta.url);
    const loaded = require(specifier) as Record<string, unknown> & { default?: YydbNative };
    return (loaded.default ?? loaded) as YydbNative;
}

/** Load the platform native binding used by the `yydb` CLI. */
export function loadNative(): YydbNative {
    const fromEnv = process.env.YYDB_NATIVE?.trim();
    if (fromEnv) {
        const bindingPath = path.resolve(fromEnv);
        if (!fs.existsSync(bindingPath)) {
            throw new Error(`YYDB_NATIVE points to missing binding: ${fromEnv}`);
        }
        return loadModule(bindingPath);
    }

    const key = platformKey();
    if (!key) {
        throw new Error(`YYDB native binding not available for ${process.platform}-${process.arch}`);
    }

    const pkg = `@yydb/yydb-${key}`;
    try {
        return loadModule(pkg);
    } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        throw new Error(
            `YYDB native binding not found. Reinstall @yydb/yydb with optional dependencies, set YYDB_NATIVE, or stage ${pkg}. ${message}`,
        );
    }
}
