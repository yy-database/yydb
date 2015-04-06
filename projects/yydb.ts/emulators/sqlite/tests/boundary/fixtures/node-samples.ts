import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";

import type { TempFixture } from "./shared.ts";

const SAMPLES_DDL =
    "CREATE TABLE samples (id INTEGER PRIMARY KEY, quantity INTEGER, score REAL, label TEXT, payload BLOB, note TEXT)";

/** Upstream SQLite file used to exercise reader validation and scan limits. */
export function createNodeSamplesFixture(): TempFixture & { bytes: Buffer } {
    const directory = mkdtempSync(join(tmpdir(), "yydb-sqlite-boundary-"));
    const path = join(directory, "samples.sqlite");
    const database = new DatabaseSync(path);
    try {
        database.exec(SAMPLES_DDL);
        database.exec(
            "INSERT INTO samples VALUES (1, 42, 3.5, 'alpha', x'00ff10', NULL)",
        );
        database.exec(
            "INSERT INTO samples VALUES (2, -7, -0.25, 'beta', x'ff', 'second')",
        );
    } finally {
        database.close();
    }
    return { directory, path, bytes: readFileSync(path) };
}
