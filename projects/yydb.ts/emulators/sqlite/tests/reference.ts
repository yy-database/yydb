import { spawnSync } from "node:child_process";

/**
 * Python interpreter that carries the standard-library `sqlite3` module.
 *
 * Mirrors the Rust interop helper: `YYDS_SQLITE_REFERENCE_PYTHON` wins, then
 * `python3`, then `python`. Interop cases stay conditional because the upstream
 * engine is an external tool, not a repository dependency.
 */
export function referencePython(): string | undefined {
    const configured = process.env.YYDS_SQLITE_REFERENCE_PYTHON?.trim();
    const candidates = configured ? [configured] : ["python3", "python"];
    return candidates.find((candidate) => {
        const probe = spawnSync(candidate, ["-c", "import sqlite3"], { timeout: 5_000 });
        return probe.status === 0;
    });
}
