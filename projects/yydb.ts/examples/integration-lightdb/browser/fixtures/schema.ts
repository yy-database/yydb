export const CASE_ROW_SCHEMA = `table CaseRow {
    @@id: i64,
    label: utf8,
}`;

export const SIMPLE_SCHEMA = `table Simple {
    @@id: i64,
    name: utf8,
    note: utf8,
}`;

/** VOS programs produced by converting an upstream SQLite `案例` table. */
export const CASE_ADOPTION_PROGRAMS = [
    'CaseRow { id: 10, label: "导入-A" }.insert()',
    'CaseRow { id: 11, label: "导入-B" }.insert()',
] as const;

/** VOS programs produced from the reference corpus `simple` table. */
export const SIMPLE_ADOPTION_PROGRAMS = [
    'Simple { id: 1, name: "alpha", note: null }.insert()',
    'Simple { id: 2, name: "00042", note: "" }.insert()',
    'Simple { id: 3, name: "中文", note: "note" }.insert()',
] as const;

export const SQL_SAMPLES = [
    "SELECT 1",
    "SELECT id, label FROM CaseRow",
    "INSERT INTO CaseRow (id, label) VALUES (1, '第一')",
    "CREATE TABLE CaseRow (id INTEGER PRIMARY KEY, label TEXT)",
    "PRAGMA schema_version",
    "DELETE FROM CaseRow WHERE id = 1",
    "SELECT a.* FROM Simple a JOIN CaseRow b ON a.id = b.id",
] as const;
