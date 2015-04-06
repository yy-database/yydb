import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";

import type { TempFixture } from "./shared.ts";

/** Node-built database that exercises reader limitations from upstream SQLite. */
export function createLimitationFixture(): TempFixture & { bytes: Buffer } {
    const directory = mkdtempSync(join(tmpdir(), "yydb-sqlite-limits-"));
    const path = join(directory, "limits.sqlite");
    const database = new DatabaseSync(path);
    try {
        database.exec("CREATE TABLE rowid_table (id INTEGER PRIMARY KEY, value TEXT)");
        database.exec("INSERT INTO rowid_table VALUES (1, 'ok')");
        database.exec("CREATE TABLE wo (k TEXT PRIMARY KEY, v INTEGER) WITHOUT ROWID");
        database.exec("INSERT INTO wo VALUES ('k1', 1)");
        database.exec("CREATE VIEW v_rowid AS SELECT id, value FROM rowid_table");
        database.exec("CREATE VIRTUAL TABLE virt USING rtree(id, minX, maxX)");
    } finally {
        database.close();
    }
    return { directory, path, bytes: readFileSync(path) };
}
