import { createRequire } from 'node:module';
import fs from 'node:fs';
import path from 'node:path';
import { platformKey } from './resolve.js';

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
}

/** Atomic operations exported by the Node-API binding. */
export interface YydbNative {
    version(): string;
    serve(dbPath: string, bind: string, insecureBind?: boolean): void;
    Database: { open(dbPath: string): NativeDatabase };
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
