import { SqliteDatabase } from "@yydb/sqlite";

import type { CaseRow } from "./case.ts";

const READ_LIMITS = { maxPages: 64, maxRows: 32 } as const;

/** Reads the upstream SQLite `案例` table through the snapshot reader only. */
export function readCaseRowsFromSqlite(sqlitePath: string): CaseRow[] {
    const database = SqliteDatabase.open(sqlitePath);
    return database.table("案例", READ_LIMITS).map((row) => ({
        rowid: Number(row.rowid),
        label: row.columns[1].valueText ?? "",
    }));
}

export function vosInsertCaseRow(row: CaseRow): string {
    const label = JSON.stringify(row.label);
    return `CaseRow { id: ${row.rowid}, label: ${label} }.insert()`;
}

/** Converts SQLite storage bytes into VOS write programs for @yydb/yydb. */
export function buildCaseAdoptionProgram(sqlitePath: string): string[] {
    return readCaseRowsFromSqlite(sqlitePath).map(vosInsertCaseRow);
}
