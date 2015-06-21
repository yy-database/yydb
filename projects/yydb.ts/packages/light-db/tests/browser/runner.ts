import * as lightDb from "@yydb/light-db";
import { Database } from "@yydb/light-db";
import { initWasm } from "@yydb/yydb-unknown-wasm32";

import {
    CASE_ADOPTION_PROGRAMS,
    CASE_ROW_SCHEMA,
    SIMPLE_ADOPTION_PROGRAMS,
    SIMPLE_SCHEMA,
    SQL_SAMPLES,
} from "./fixtures/schema.ts";

export interface BrowserTestResult {
    name: string;
    ok: boolean;
    error?: string;
}

const results: BrowserTestResult[] = [];

async function run(name: string, fn: () => Promise<void>): Promise<void> {
    try {
        await fn();
        results.push({ name, ok: true });
    } catch (error) {
        results.push({ name, ok: false, error: error instanceof Error ? error.message : String(error) });
    }
}

async function rejectSql(database: Database): Promise<void> {
    for (const sql of SQL_SAMPLES) {
        let rejected = false;
        try {
            await database.query(sql);
        } catch {
            rejected = true;
        }
        if (!rejected) {
            throw new Error(`query must reject SQL: ${sql}`);
        }

        rejected = false;
        try {
            await database.execute(sql);
        } catch {
            rejected = true;
        }
        if (!rejected) {
            throw new Error(`execute must reject SQL: ${sql}`);
        }
    }
}

async function main(): Promise<void> {
    // No explicit module: the binding resolves its own bundled `yydb_wasm_bg.wasm`
    // asset, which Vite fingerprints into `dist/assets`.
    await initWasm();

    await run("@yydb/light-db rejects SQL and only accepts VOS programs", async () => {
        const database = await Database.open("sql-negative", { memory: true });
        try {
            await database.ensureSchema(CASE_ROW_SCHEMA);
            await rejectSql(database);
            await database.execute('CaseRow { id: 9, label: "wasm-vos" }.insert()');
            const rows = await database.query("CaseRow.all().collect()");
            if (JSON.stringify(rows) !== JSON.stringify([{ id: 9, label: "wasm-vos" }])) {
                throw new Error(`unexpected rows: ${JSON.stringify(rows)}`);
            }
        } finally {
            await database.close();
        }
    });

    await run("@yydb/light-db does not expose the SQLite snapshot reader", async () => {
        if ("SqliteDatabase" in lightDb || "SqliteSnapshot" in lightDb) {
            throw new Error("light-db must not re-export the SQLite reader");
        }
        const proto = Database.prototype as unknown as Record<string, unknown>;
        for (const method of ["openSqlite", "importSqlite", "readSqlite", "table", "schema"]) {
            if (method in proto) {
                throw new Error(`light-db must not implement ${method}`);
            }
        }
    });

    await run("@yydb/light-db prepare rejects SQL but accepts VOS", async () => {
        const database = await Database.open("prepare-negative", { memory: true });
        try {
            await database.ensureSchema(CASE_ROW_SCHEMA);
            let rejected = false;
            try {
                await database.prepare("SELECT id FROM CaseRow");
            } catch {
                rejected = true;
            }
            if (!rejected) {
                throw new Error("prepare must reject SQL");
            }

            const statement = await database.prepare("CaseRow.all().collect()");
            const rows = await statement.execute({});
            if (!Array.isArray(rows) || rows.length !== 0) {
                throw new Error("prepared VOS read must start empty");
            }
            statement.close();
        } finally {
            await database.close();
        }
    });

    await run("@yydb/light-db transactions commit adopted rows", async () => {
        const database = await Database.open("transaction-adoption", { memory: true });
        try {
            await database.ensureSchema(CASE_ROW_SCHEMA);
            await database.transaction(async (tx) => {
                await tx.execute('CaseRow { id: 20, label: "tx-a" }.insert()');
                await tx.execute('CaseRow { id: 21, label: "tx-b" }.insert()');
            });
            const rows = (await database.query("CaseRow.all().collect()") as Array<{ id: number }>)
                .map((row) => row.id)
                .sort((left, right) => left - right);
            if (JSON.stringify(rows) !== JSON.stringify([20, 21])) {
                throw new Error(`unexpected transaction rows: ${JSON.stringify(rows)}`);
            }
        } finally {
            await database.close();
        }
    });

    await run("@yydb/light-db doctor reports a fresh in-memory session", async () => {
        const database = await Database.open("doctor-smoke", { memory: true });
        try {
            await database.ensureSchema(SIMPLE_SCHEMA);
            const report = await database.doctor();
            if (!report || typeof report.last_commit_sequence !== "number") {
                throw new Error("doctor report must include commit sequence metadata");
            }
            if (!Array.isArray(report.namespaces)) {
                throw new Error("doctor report must include namespaces");
            }
        } finally {
            await database.close();
        }
    });

    await run("reference corpus simple rows reach light-db only through explicit VOS conversion", async () => {
        const database = await Database.open("simple-adoption", { memory: true });
        try {
            await database.ensureSchema(SIMPLE_SCHEMA);
            for (const program of SIMPLE_ADOPTION_PROGRAMS) {
                await database.execute(program);
            }
            const rows = (await database.query("Simple.all().collect()") as Array<{ id: number; name: string }>)
                .sort((left, right) => left.id - right.id)
                .map((row) => row.name);
            if (JSON.stringify(rows) !== JSON.stringify(["alpha", "00042", "中文"])) {
                throw new Error(`unexpected simple rows: ${JSON.stringify(rows)}`);
            }
        } finally {
            await database.close();
        }
    });

    await run("SQLite rows reach @yydb/light-db only through explicit VOS conversion", async () => {
        const database = await Database.open("sqlite-adoption", { memory: true });
        try {
            await database.ensureSchema(CASE_ROW_SCHEMA);
            const before = await database.query("CaseRow.all().collect()");
            if (before.length !== 0) {
                throw new Error("opening light-db must not auto-import SQLite bytes");
            }

            for (const program of CASE_ADOPTION_PROGRAMS) {
                await database.execute(program);
            }

            const rows = (await database.query("CaseRow.all().collect()") as Array<{ id: number; label: string }>)
                .sort((left, right) => left.id - right.id);
            const expected = [
                { id: 10, label: "导入-A" },
                { id: 11, label: "导入-B" },
            ];
            if (JSON.stringify(rows) !== JSON.stringify(expected)) {
                throw new Error(`unexpected adopted rows: ${JSON.stringify(rows)}`);
            }
        } finally {
            await database.close();
        }
    });

    window.__yydbLightDbResults = results;
}

declare global {
    interface Window {
        __yydbLightDbResults?: BrowserTestResult[];
    }
}

void main();
