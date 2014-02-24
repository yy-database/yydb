import { createRequire } from "node:module";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { nativeBindingName, platformKey, type PlatformKey } from "./resolve-bin.js";

export interface YydbNative {
    version(): string;
    serve(dbPath: string, bind: string, insecureBind?: boolean): void;
    initDb(dbPath: string, schemaVersion: number, schemaDocument?: string | null): void;
    infoText(dbPath: string): string;
    readSchemaFile(schemaPath: string): string;
}

function existsFile(filePath: string): boolean {
    try {
        fs.accessSync(filePath, fs.constants.F_OK);
        return true;
    } catch {
        return false;
    }
}

function loadFromPath(bindingPath: string): YydbNative {
    const require = createRequire(import.meta.url);
    return require(bindingPath) as YydbNative;
}

function monorepoBindingPath(key: PlatformKey): string | null {
    const here = path.dirname(fileURLToPath(import.meta.url));
    const repoRoot = path.resolve(here, "../../..");
    const profile = process.env.YYDB_NATIVE_PROFILE === "debug" ? "debug" : "release";
    const base = path.join(repoRoot, "target", profile);
    const candidates =
        process.platform === "win32"
            ? [path.join(base, "yydb_napi.dll"), path.join(base, "yydb_napi.node")]
            : [
                  path.join(base, "libyydb_napi.so"),
                  path.join(base, "libyydb_napi.dylib"),
                  path.join(base, "yydb_napi.node"),
              ];
    for (const candidate of candidates) {
        if (existsFile(candidate)) {
            return candidate;
        }
    }
    const staged = path.join(
        repoRoot,
        "projects",
        "packages",
        `yydb-${key}`,
        nativeBindingName(key),
    );
    return existsFile(staged) ? staged : null;
}

/** Load the platform native binding used by the `yydb` CLI. */
export function loadNative(): YydbNative {
    const fromEnv = process.env.YYDB_NATIVE?.trim();
    if (fromEnv) {
        if (!existsFile(fromEnv)) {
            throw new Error(`YYDB_NATIVE points to missing binding: ${fromEnv}`);
        }
        return loadFromPath(path.resolve(fromEnv));
    }

    const key = platformKey();
    if (key) {
        const pkg = `@yydb/yydb-${key}`;
        const require = createRequire(import.meta.url);
        try {
            const pkgJson = require.resolve(`${pkg}/package.json`);
            const candidate = path.join(path.dirname(pkgJson), nativeBindingName(key));
            if (existsFile(candidate)) {
                return loadFromPath(candidate);
            }
        } catch {
            // optionalDependency not installed for this platform
        }
        const local = monorepoBindingPath(key);
        if (local) {
            return loadFromPath(local);
        }
    }

    throw new Error(
        "YYDB native binding not found. Install the matching optionalDependency " +
            `(@yydb/yydb-${key ?? "<platform>"}), set YYDB_NATIVE, or build ` +
            "`cargo build -p yydb-napi --release` in this repo.",
    );
}
