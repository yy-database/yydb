import { createRequire } from "node:module";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);

export type PlatformKey = "win32-x64" | "linux-x64" | "darwin-x64" | "darwin-arm64";

/** Map Node platform/arch to optionalDependency package suffix. */
export function platformKey(platform = process.platform, arch = process.arch): PlatformKey | null {
    if (platform === "win32" && arch === "x64") return "win32-x64";
    if (platform === "linux" && arch === "x64") return "linux-x64";
    if (platform === "darwin" && arch === "x64") return "darwin-x64";
    if (platform === "darwin" && arch === "arm64") return "darwin-arm64";
    return null;
}

function binaryName(platform = process.platform): string {
    return platform === "win32" ? "yydb.exe" : "yydb";
}

function existsExecutable(filePath: string): boolean {
    try {
        fs.accessSync(filePath, fs.constants.F_OK);
        return true;
    } catch {
        return false;
    }
}

/**
 * Resolve the `yydb` engine binary without asking the app developer to
 * configure downloads. Order: `YYDB_BIN` → optional platform package →
 * cargo `target/{release,debug}` (monorepo).
 */
export function resolveYydbBinary(): string {
    const fromEnv = process.env.YYDB_BIN?.trim();
    if (fromEnv) {
        if (!existsExecutable(fromEnv)) {
            throw new Error(`YYDB_BIN points to missing binary: ${fromEnv}`);
        }
        return path.resolve(fromEnv);
    }

    const key = platformKey();
    if (key) {
        const pkg = `@yydb/yydb-${key}`;
        const name = binaryName();
        try {
            const pkgJson = require.resolve(`${pkg}/package.json`);
            const candidate = path.join(path.dirname(pkgJson), name);
            if (existsExecutable(candidate)) {
                return candidate;
            }
        } catch {
            // optionalDependency not installed for this platform
        }
    }

    const here = path.dirname(fileURLToPath(import.meta.url));
    const repoRoot = path.resolve(here, "../../..");
    for (const profile of ["release", "debug"] as const) {
        const candidate = path.join(repoRoot, "target", profile, binaryName());
        if (existsExecutable(candidate)) {
            return candidate;
        }
    }

    throw new Error(
        "YYDB engine binary not found. Install the matching optionalDependency " +
            `(@yydb/yydb-${key ?? "<platform>"}), set YYDB_BIN, or build ` +
            "`cargo build -p yydb-tools` in this repo.",
    );
}
