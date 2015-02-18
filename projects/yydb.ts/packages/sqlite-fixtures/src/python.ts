import { spawnSync } from "node:child_process";

/** Resolves a Python interpreter with the standard-library `sqlite3` module. */
export function referencePython(): string | undefined {
    const configured = process.env.YYDS_SQLITE_REFERENCE_PYTHON?.trim();
    const candidates = configured ? [configured] : ["python3", "python"];
    return candidates.find((candidate) => {
        const probe = spawnSync(candidate, ["-c", "import sqlite3"], { timeout: 5_000 });
        return probe.status === 0;
    });
}
