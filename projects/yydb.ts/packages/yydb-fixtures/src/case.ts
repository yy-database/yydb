import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";

export const CASE_DDL = 'CREATE TABLE "案例" (id INTEGER PRIMARY KEY, label TEXT)';

export const CASE_ROW_SCHEMA = `table CaseRow {
    @@id: i64,
    label: utf8,
}`;

export interface CaseRow {
    rowid: number;
    label: string;
}

export interface CaseFixture {
    directory: string;
    path: string;
    bytes: Buffer;
}

/** Upstream SQLite file that @yydb/yydb must adopt instead of opening directly. */
export function createCaseSqliteFixture(rows?: readonly CaseRow[]): CaseFixture {
    const directory = mkdtempSync(join(tmpdir(), "yydb-adopt-case-"));
    const path = join(directory, "case.sqlite");
    const database = new DatabaseSync(path);
    try {
        database.exec(CASE_DDL);
        const seed = rows ?? [
            { rowid: 1, label: "第一" },
            { rowid: 2, label: "第二" },
        ];
        const insert = database.prepare('INSERT INTO "案例" (rowid, label) VALUES (?, ?)');
        for (const row of seed) {
            insert.run(row.rowid, row.label);
        }
    } finally {
        database.close();
    }
    return { directory, path, bytes: readFileSync(path) };
}
