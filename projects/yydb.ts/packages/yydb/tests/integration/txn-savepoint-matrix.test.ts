import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { Database } from "@yydb/yydb";

/**
 * Binding-level mirror of the Rust savepoint matrix. Raw commit/rollback
 * error paths without an open transaction stay Rust-side because the host
 * API only exposes transactions through `Database.transaction`.
 */
test("savepoint stack: chains, consumption, release and name reuse", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-sp-matrix-"));
    try {
        const database = await Database.open(join(directory, "matrix.yydb"));
        await database.ensureSchema("table T { @@id: i64, v: utf8 }");

        await database.transaction(async (tx) => {
            await tx.put("baseline", "1");

            await tx.savepoint("x");
            await tx.put("after_x", "2");
            await tx.savepoint("y");
            await tx.put("after_y", "3");

            // Duplicate names are refused while the name is on the stack.
            await assert.rejects(tx.savepoint("x"), /already exists|duplicate/i, "duplicate savepoint name");

            // Rollback-to y undoes after_y, keeps the prefix, consumes y.
            await tx.rollbackToSavepoint("y");
            assert.equal(Buffer.from(await tx.get("baseline")).toString("utf8"), "1");
            assert.equal(Buffer.from(await tx.get("after_x")).toString("utf8"), "2");
            assert.equal(await tx.get("after_y"), null);
            await assert.rejects(tx.rollbackToSavepoint("y"), /does not exist/i, "rollback-to consumes the name");

            // Rollback-to x undoes after_x as well.
            await tx.rollbackToSavepoint("x");
            assert.equal(await tx.get("after_x"), null);
            await assert.rejects(tx.rollbackToSavepoint("x"), /does not exist/i, "x was consumed");

            // RELEASE frees the name without undoing writes.
            await tx.savepoint("sp");
            await tx.put("after_release", "4");
            await tx.releaseSavepoint("sp");
            assert.equal(Buffer.from(await tx.get("after_release")).toString("utf8"), "4");
            await assert.rejects(tx.rollbackToSavepoint("sp"), /does not exist/i, "released name rejected");
            await tx.savepoint("sp");

            // A full rollback undoes everything, including post-release writes.
            throw new Error("trigger full rollback");
        }).then(
            () => assert.fail("the rollback trigger must propagate"),
            (error) => assert.match(String(error), /trigger full rollback/),
        );
        assert.equal(await database.get("baseline"), null, "full rollback discards every write");

        // Commit clears the savepoint stack; fresh transactions start clean.
        await database.transaction(async (tx) => {
            await tx.put("kept", "5");
        });
        assert.equal(Buffer.from(await database.get("kept")).toString("utf8"), "5", "commit persists writes");
        await database.transaction(async (tx) => {
            await assert.rejects(tx.rollbackToSavepoint("kept"), /does not exist|savepoint/i, "stack cleared by commit");
        });

        await database.close();
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("release of an outer savepoint does not disturb inner ones", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-sp-inner-"));
    try {
        const database = await Database.open(join(directory, "inner.yydb"));
        await database.ensureSchema("table T { @@id: i64, v: utf8 }");
        await database.transaction(async (tx) => {
            await tx.savepoint("outer");
            await tx.put("w", "1");
            await tx.savepoint("inner");
            await tx.put("v", "2");
            await tx.releaseSavepoint("outer");

            // `inner` is still reachable and undoes only writes after it.
            await tx.rollbackToSavepoint("inner");
            assert.equal(Buffer.from(await tx.get("w")).toString("utf8"), "1");
            assert.equal(await tx.get("v"), null);
        });
        assert.equal(Buffer.from(await database.get("w")).toString("utf8"), "1", "prefix write survives the commit");
        assert.equal(await database.get("v"), null, "rolled-back write stays gone");
        await database.close();
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("querying a schemaless database is refused instead of crashing", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-schemaless-"));
    try {
        const database = await Database.open(join(directory, "empty.yydb"));
        await assert.rejects(database.query("T.all().collect()"), /schema/i, "query before schema is refused");
        await database.close();
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});


