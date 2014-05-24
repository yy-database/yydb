import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export type PlatformKey = 'win32-x64' | 'linux-x64' | 'darwin-x64' | 'darwin-arm64';

/** Map Node platform/arch to optionalDependency package suffix. */
export function platformKey(platform = process.platform, arch = process.arch): PlatformKey | null {
    if (platform === 'win32' && arch === 'x64') return 'win32-x64';
    if (platform === 'linux' && arch === 'x64') return 'linux-x64';
    if (platform === 'darwin' && arch === 'x64') return 'darwin-x64';
    if (platform === 'darwin' && arch === 'arm64') return 'darwin-arm64';
    return null;
}

/** Native binding filename inside each `@yydb/yydb-<platform>` package `lib/`. */
export function nativeBindingName(key: PlatformKey): string {
    switch (key) {
        case 'win32-x64':
            return 'yydb-win32-x64-msvc.node';
        case 'linux-x64':
            return 'yydb-linux-x64-gnu.node';
        case 'darwin-x64':
            return 'yydb-darwin-x64.node';
        case 'darwin-arm64':
            return 'yydb-darwin-arm64.node';
    }
}

function existsFile(filePath: string): boolean {
    try {
        fs.accessSync(filePath, fs.constants.F_OK);
        return true;
    } catch {
        return false;
    }
}

/**
 * Resolve the TypeScript `yydb` CLI entry script.
 * Order: `YYDB_CLI` → this package `dist/cli.js`.
 */
export function resolveYydbCli(): string {
    const fromEnv = process.env.YYDB_CLI?.trim();
    if (fromEnv) {
        if (!existsFile(fromEnv)) {
            throw new Error(`YYDB_CLI points to missing script: ${fromEnv}`);
        }
        return path.resolve(fromEnv);
    }
    const here = path.dirname(fileURLToPath(import.meta.url));
    const candidate = path.join(here, 'cli.js');
    if (!existsFile(candidate)) {
        throw new Error('YYDB CLI script not found. Build `@yydb/yydb` (`pnpm --filter @yydb/yydb build`) or set YYDB_CLI.');
    }
    return candidate;
}

/**
 * @deprecated Use {@link resolveYydbCli}. Kept for callers that still import this name.
 */
export function resolveYydbBinary(): string {
    return resolveYydbCli();
}
