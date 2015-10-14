import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import test from "node:test";

import { Database } from "@yydb/yydb";
import { runCli } from "../../dist/cli/cli.js";

/**
 * Product commands: `restore` (verified backup copy), and the explicit
 * SQLite ETL pair `export` / `import` over the declared
 * `(key TEXT PRIMARY KEY, value BLOB)` subset.
 */

const DRIVER = resolve(import.meta.dirname, "../../../sqlite-fixtures/node_modules/better-sqlite3");
process.env.YYDB_SQLITE_DRIVER = DRIVER;

interface Row {
    key: string;
    value: Uint8Array;
}

interface SqliteHandle {
    prepare(sql: string): { all(...params: unknown[]): unknown[]; run(...params: unknown[]): unknown };
    exec(sql: string): void;
    close(): void;
}

function withTemporary(run: (directory: string) => Promise<void>): Promise<void> {
    const directory = mkdtempSync(join(tmpdir(), "yydb-sqlite-etl-"));
    return run(directory).finally(() => rmSync(directory, { recursive: true, force: true }));
}

function openSqlite(file: string): SqliteHandle {
    const require = createRequire(DRIVER);
    const driver = require(DRIVER) as new (path: string) => SqliteHandle;
    return new driver(file);
}

async function seed(file: string): Promise<void> {
    const database = await Database.open(file);
    try {
        await database.put("alpha", "one");
        await database.put("beta", "two");
        await database.put("gamma", "three");
    } finally {
        await database.close();
    }
}

test("export writes a records-shaped SQLite table", async () => {
    await withTemporary(async (directory) => {
        const databasePath = join(directory, "source.yydb");
        const sqlitePath = join(directory, "export.sqlite");
        await seed(databasePath);

        assert.equal(await runCli(["export", databasePath, "--sqlite", sqlitePath]), 0);

        const sqlite = openSqlite(sqlitePath);
        try {
            const rows = sqlite.prepare("SELECT key, value FROM records ORDER BY key").all() as Row[];
            assert.deepEqual(rows.map((row) => row.key), ["alpha", "beta", "gamma"]);
            assert.equal(Buffer.from(rows[0]?.value ?? []).toString(), "one");
        } finally {
            sqlite.close();
        }
    });
});

test("import reads the same declared subset into a fresh database", async () => {
    await withTemporary(async (directory) => {
        const sourcePath = join(directory, "source.yydb");
        const sqlitePath = join(directory, "export.sqlite");
        const targetPath = join(directory, "imported.yydb");
        await seed(sourcePath);
        assert.equal(await runCli(["export", sourcePath, "--sqlite", sqlitePath]), 0);
        assert.equal(await runCli(["import", sqlitePath, "--into", targetPath]), 0);

        const database = await Database.open(targetPath);
        try {
            assert.equal(new TextDecoder().decode(await database.get("alpha") ?? new Uint8Array()), "one");
            assert.equal(new TextDecoder().decode(await database.get("gamma") ?? new Uint8Array()), "three");
        } finally {
            await database.close();
        }
    });
});

test("import refuses a table outside the declared subset", async () => {
    await withTemporary(async (directory) => {
        const sqlitePath = join(directory, "shaped.sqlite");
        const targetPath = join(directory, "target.yydb");
        const sqlite = openSqlite(sqlitePath);
        try {
            sqlite.exec("CREATE TABLE shaped (name TEXT, payload BLOB)");
        } finally {
            sqlite.close();
        }
        assert.equal(await runCli(["import", sqlitePath, "--table", "shaped", "--into", targetPath]), 1);
    });
});

test("restore verifies a backup and copies it to a target", async () => {
    await withTemporary(async (directory) => {
        const sourcePath = join(directory, "source.yydb");
        const backupPath = join(directory, "backup.yydb");
        const targetPath = join(directory, "restored.yydb");
        await seed(sourcePath);

        const source = await Database.open(sourcePath);
        try {
            await source.backup(backupPath);
        } finally {
            await source.close();
        }

        assert.equal(await runCli(["restore", backupPath]), 0);
        assert.equal(await runCli(["restore", backupPath, "--target", targetPath]), 0);

        const restored = await Database.restore(targetPath);
        try {
            assert.equal(new TextDecoder().decode(await restored.get("beta") ?? new Uint8Array()), "two");
        } finally {
            await restored.close();
        }
    });
});
