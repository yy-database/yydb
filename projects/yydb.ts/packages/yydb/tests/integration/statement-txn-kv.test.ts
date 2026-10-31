import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { Database, YydbError } from "@yydb/yydb";

/**
 * Session usage surface: prepared statements and their bind/close lifecycle,
 * explicit transactions with commit and rollback, and the raw key/value layer.
 */

const SCHEMA = `table P {
    @@p_id: i64,
    name: utf8,
    score: i64,
    active: bool,
    ratio: f64,
}`;

async function withDatabase(run: (database: Database) => Promise<void>): Promise<void> {
    const directory = mkdtempSync(join(tmpdir(), "yydb-stmt-txn-kv-"));
    try {
        const database = await Database.open(join(directory, "session.yydb"));
        try {
            await database.ensureSchema(SCHEMA);
            await database.execute("P::insert({ p_id: $id, name: $n, score: $s, active: $a })", { id: 1, n: "delta", s: 30, a: true });
            await database.execute("P::insert({ p_id: $id, name: $n, score: $s, active: $a })", { id: 2, n: "alpha", s: 10, a: false });
            await database.execute("P::insert({ p_id: $id, name: $n, score: $s, active: $a })", { id: 3, n: "charlie", s: 20, a: true });
            await run(database);
        } finally {
            await database.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
}

async function rejectDiagnostic(run: () => Promise<unknown>): Promise<YydbError> {
    let caught: unknown;
    try {
        await run();
    } catch (cause) {
        caught = cause;
    }
    assert.ok(caught, "expected the call to reject");
    if (caught instanceof YydbError) {
        return caught;
    }
    assert.ok(caught instanceof Error, `unexpected rejection: ${String(caught)}`);
    const diagnostic = YydbError.fromDiagnosticJson(caught.message);
    assert.ok(diagnostic, `rejection must carry a diagnostic envelope: ${caught.message}`);
    return diagnostic;
}

test("a prepared statement exposes its plan metadata and reuses across executions", async () => {
    await withDatabase(async (database) => {
        const source = "P.all().filter(x => x.active).take($limit).collect()";
        const statement = await database.prepare(source);
        try {
            assert.equal(statement.isClosed, false);
            assert.match(statement.statementId, /^stmt-[0-9a-f]+$/);
            assert.deepEqual(statement.parameters, [{ name: "limit", index: 0, slotTy: "any", usageCount: 1 }]);

            const info = statement.info as Record<string, unknown>;
            assert.equal(info.dialect, "Vos");
            assert.equal(info.source, source);
            assert.equal(info.queryKind, "Read");
            assert.deepEqual(info.readSet, ["P"]);
            assert.deepEqual(info.writeSet, []);
            assert.equal(info.statementId, statement.statementId);

            const first = await statement.execute<{ p_id: number }>({ limit: 1 });
            assert.equal(first.length, 1);
            const second = await statement.execute<{ p_id: number }>({ limit: 2 });
            assert.deepEqual(second.map((row) => row.p_id), [1, 3]);
        } finally {
            statement.close();
        }
    });
});

test("bind failures report `yydb.query.bind` with the offending parameter", async () => {
    await withDatabase(async (database) => {
        const statement = await database.prepare("P.all().take($limit).collect()");
        try {
            const missing = await rejectDiagnostic(() => statement.execute({}));
            assert.ok(missing instanceof YydbError);
            assert.equal(missing.code, "yydb.query.bind");
            assert.equal(missing.params?.parameter, "limit");

            const extra = await rejectDiagnostic(() => statement.execute({ limit: 1, nope: 2 }));
            assert.equal(extra.code, "yydb.query.bind");
            assert.equal(extra.params?.parameter, "nope");
        } finally {
            statement.close();
        }
    });
});

test("a non-integer window size reaches the engine as a schema rejection", async () => {
    await withDatabase(async (database) => {
        const statement = await database.prepare("P.all().take($limit).collect()");
        try {
            const rejection = await rejectDiagnostic(() => statement.execute({ limit: "two" }));
            assert.equal(rejection.code, "yydb.schema.invalid");
            assert.match(rejection.message, /LIMIT\/OFFSET count must be an integer/);
        } finally {
            statement.close();
        }
    });
});

test("non-finite numbers are refused at the host bind boundary", async () => {
    await withDatabase(async (database) => {
        const rejection = await rejectDiagnostic(() =>
            database.execute("P::insert({ p_id: 8, name: $n, ratio: $r })", { n: "nan", r: Number.NaN }),
        );
        assert.ok(rejection instanceof YydbError);
        assert.equal(rejection.code, "yydb.query.bind");
    });
});

test("closing a statement is idempotent and later executions fail closed", async () => {
    await withDatabase(async (database) => {
        const statement = await database.prepare("P.all().collect()");
        statement.close();
        statement.close();
        assert.equal(statement.isClosed, true);
        const rejection = await rejectDiagnostic(() => statement.execute());
        assert.equal(rejection.code, "yydb.statement.closed");
    });
});

test("preparing a write program is refused before the statement exists", async () => {
    await withDatabase(async (database) => {
        const rejection = await rejectDiagnostic(() => database.prepare('P::insert({ p_id: 7, name: "seven" })'));
        assert.equal(rejection.code, "yydb.schema.invalid");
        assert.match(rejection.message, /pipelines ending in `\.collect\(\)`/);
    });
});

test("a transaction commits writes when the callback returns normally", async () => {
    await withDatabase(async (database) => {
        const result = await database.transaction(async (tx) => {
            await tx.put("t/committed", "yes");
            return 42;
        });
        assert.equal(result, 42);
        const value = await database.get("t/committed");
        assert.ok(value);
        assert.equal(Buffer.from(value).toString("utf8"), "yes");
    });
});

test("a throwing callback rolls back and the original error propagates", async () => {
    await withDatabase(async (database) => {
        await assert.rejects(
            database.transaction(async (tx) => {
                await tx.put("t/rolled-back", "ghost");
                throw new Error("boom-rollback");
            }),
            /boom-rollback/,
        );
        assert.equal(await database.get("t/rolled-back"), null);
    });
});

test("the transaction handle observes its own uncommitted writes", async () => {
    await withDatabase(async (database) => {
        await assert.rejects(
            database.transaction(async (tx) => {
                await tx.put("t/inside", "x");
                const inside = await tx.get("t/inside");
                assert.ok(inside);
                assert.equal(Buffer.from(inside).toString("utf8"), "x");
                throw new Error("rollback-after-read");
            }),
            /rollback-after-read/,
        );
        assert.equal(await database.get("t/inside"), null);
    });
});

test("nested transactions are refused while one is open", async () => {
    await withDatabase(async (database) => {
        await assert.rejects(
            database.transaction(async () => database.transaction(async (tx) => tx.put("n/inner", "y"))),
            /transaction already open/,
        );
        assert.equal(await database.get("n/inner"), null);
    });
});

test("the key/value layer covers absent reads, silent deletes and octet-exact roundtrips", async () => {
    await withDatabase(async (database) => {
        assert.equal(await database.get("kv/missing"), null);
        await database.delete("kv/missing");

        await database.put("kv/text", "hello");
        const text = await database.get("kv/text");
        assert.ok(text instanceof Uint8Array);
        assert.equal(Buffer.from(text).toString("utf8"), "hello");

        const octets = Uint8Array.from([0x00, 0x80, 0xff]);
        await database.put("kv/raw", octets);
        const raw = await database.get("kv/raw");
        assert.deepEqual(Array.from(raw ?? []), Array.from(octets));

        await database.put("kv/empty", new Uint8Array());
        const empty = await database.get("kv/empty");
        assert.ok(empty);
        assert.equal(empty.length, 0);
    });
});

test("scan walks keys in lexical order with prefix, cursor and limit", async () => {
    await withDatabase(async (database) => {
        await database.put("s/1", "one");
        await database.put("s/2", "two");
        await database.put("s/3", "three");

        const prefixed = await database.scan({ prefix: "s/" });
        assert.deepEqual(prefixed.map((row) => row.key), ["s/1", "s/2", "s/3"]);

        const after = await database.scan({ prefix: "s/", after: "s/1" });
        assert.deepEqual(after.map((row) => row.key), ["s/2", "s/3"]);

        const limited = await database.scan({ prefix: "s/", limit: 2 });
        assert.deepEqual(limited.map((row) => row.key), ["s/1", "s/2"]);

        const decoded = await database.scan({ prefix: "s/", after: "s/2" });
        assert.equal(Buffer.from(decoded[0].value).toString("utf8"), "three");
    });
});

test("batch applies puts and deletes in one change set", async () => {
    await withDatabase(async (database) => {
        await database.put("b/gone", "bye");
        await database.batch([
            { key: "b/one", value: "1" },
            { key: "b/two", value: Uint8Array.from([2]) },
            { key: "b/gone", value: null },
        ]);
        assert.ok(await database.get("b/one"));
        assert.deepEqual(Array.from((await database.get("b/two")) ?? []), [2]);
        assert.equal(await database.get("b/gone"), null);
    });
});

test("compareExchange creates, replaces and deletes only on exact expectations", async () => {
    await withDatabase(async (database) => {
        assert.equal(await database.compareExchange("cas/key", "absent-value", "x"), false);
        assert.equal(await database.compareExchange("cas/key", null, "x"), true);
        assert.equal(Buffer.from((await database.get("cas/key")) ?? new Uint8Array()).toString("utf8"), "x");
        assert.equal(await database.compareExchange("cas/key", "stale", "y"), false);
        assert.equal(await database.compareExchange("cas/key", "x", "y"), true);
        assert.equal(Buffer.from((await database.get("cas/key")) ?? new Uint8Array()).toString("utf8"), "y");
        assert.equal(await database.compareExchange("cas/key", "y", null), true);
        assert.equal(await database.get("cas/key"), null);
    });
});

test("key/value writes survive a reopen of the same file", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-kv-reopen-"));
    const path = join(directory, "kv.yydb");
    try {
        const database = await Database.open(path);
        await database.put("persist/a", "alpha");
        await database.close();

        const reopened = await Database.open(path);
        try {
            const value = await reopened.get("persist/a");
            assert.ok(value);
            assert.equal(Buffer.from(value).toString("utf8"), "alpha");
        } finally {
            await reopened.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("operations on a closed handle fail with a closed-handle error", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-closed-"));
    try {
        const database = await Database.open(join(directory, "closed.yydb"));
        await database.close();
        await assert.rejects(database.get("any"), /database handle has been closed/);
        await assert.rejects(database.put("any", "value"), /database handle has been closed/);
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});
