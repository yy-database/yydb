import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { Database, YydbError } from "@yydb/yydb";

/**
 * Usage-side VOS program surface: writes through named parameters, the Phase-1
 * query pipeline (filters, ordering, windowing, projection) and the stable
 * rejections for spellings the engine does not accept yet.
 */

const SEED_SCHEMA = `table P {
    @@p_id: i64,
    name: utf8,
    score: i64,
    active: bool,
    ratio: f64,
}`;

const SEED_ROWS: ReadonlyArray<readonly [number, string, number, boolean]> = [
    [1, "delta", 30, true],
    [2, "alpha", 10, false],
    [3, "charlie", 20, true],
];

async function seededDatabase(run: (database: Database) => Promise<void>): Promise<void> {
    const directory = mkdtempSync(join(tmpdir(), "yydb-query-usage-"));
    try {
        const database = await Database.open(join(directory, "usage.yydb"));
        try {
            await database.ensureSchema(SEED_SCHEMA);
            for (const [id, name, score, active] of SEED_ROWS) {
                await database.execute("P::insert({ p_id: $id, name: $n, score: $s, active: $a })", {
                    id,
                    n: name,
                    s: score,
                    a: active,
                });
            }
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

test("named parameters cross the write boundary with their exact scalar shapes", async () => {
    await seededDatabase(async (database) => {
        await database.execute("P::insert({ p_id: $id, name: $n, score: $s, active: $a, ratio: $r })", {
            id: 4,
            n: "汉字🎉",
            s: 9007199254740993n,
            a: true,
            r: 0.1,
        });
        await database.execute("P::insert({ p_id: $id, name: $n, score: $s, active: $a })", {
            id: 5,
            n: "wide-negative",
            s: -9007199254740993n,
            a: false,
        });

        // Integers beyond the host-exact width cross the row wire tagged as
        // `{"$yydb":"i64","value":"..."}` and decode to `bigint`, so the full
        // 64-bit value survives the JSON round trip without rounding.
        const wideRows = await database.query<{ p_id: number; name: string; score: number | bigint }>(
            "P.all().collect()",
        );
        const wideById = new Map(wideRows.map((row) => [row.p_id, row]));
        assert.equal(wideById.get(4)?.name, "汉字🎉");
        assert.equal(wideById.get(4)?.score, 9007199254740993n);
        assert.equal(wideById.get(5)?.score, -9007199254740993n);

        // In-range integers stay bare JSON numbers and decode as `number`.
        const seededRow = wideById.get(1);
        assert.ok(seededRow, "the seeded row must be present");
        assert.equal(seededRow.score, 30);
        assert.equal(typeof seededRow.score, "number");

        // The stored row keeps the wide integer verbatim in the KV view.
        const stored = await database.scan({ prefix: "__yydb/v0/row/P/", limit: 100 });
        const row4 = stored.find((row) => Buffer.from(row.value).toString("utf8").includes("汉字"));
        assert.ok(row4, "the seeded row must be reachable through the KV view");
        assert.match(Buffer.from(row4.value).toString("utf8"), /9007199254740993/);
    });
});

test("a write program may omit fields; the stored row carries only what was set", async () => {
    await seededDatabase(async (database) => {
        await database.execute("P::insert({ p_id: 9 })");
        const rows = await database.query<Record<string, unknown>>("P.all().filter(x => x.p_id == 9).collect()");
        assert.deepEqual(rows, [{ p_id: 9 }]);
    });
});

test("re-inserting the same primary key replaces the row instead of duplicating it", async () => {
    await seededDatabase(async (database) => {
        await database.execute("P::insert({ p_id: 1, name: $n, score: $s, active: $a })", { n: "zeta", s: 1, a: false });
        const rows = await database.query<{ p_id: number; name: string }>("P.all().collect()");
        assert.equal(rows.length, SEED_ROWS.length, "upsert must not grow the table");
        assert.ok(rows.some((row) => row.p_id === 1 && row.name === "zeta"));
        assert.ok(!rows.some((row) => row.name === "delta"));
    });
});

test("a unique column collision reports a stable violation code with structured params", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-unique-"));
    try {
        const database = await Database.open(join(directory, "unique.yydb"));
        try {
            await database.ensureSchema("table E {\n    @@e_id: i64,\n    @email: utf8,\n}");
            await database.execute('E::insert({ e_id: 1, email: "a@x.io" })');
            const violation = await rejectDiagnostic(() => database.execute('E::insert({ e_id: 2, email: "a@x.io" })'));
            assert.equal(violation.code, "yydb.unique.violation");
            assert.equal(violation.operation, "query");
            assert.equal(violation.retryable, false);
            assert.equal(violation.params?.table, "E");
            assert.equal(violation.params?.columns, "email");
            assert.equal(violation.params?.constraint, "E_email_key");
        } finally {
            await database.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test("Phase-1 filters accept boolean field paths and `==` / `>` comparisons", async () => {
    await seededDatabase(async (database) => {
        const active = await database.query<{ p_id: number }>("P.all().filter(x => x.active).collect()");
        assert.deepEqual(active.map((row) => row.p_id).sort(), [1, 3]);

        const byName = await database.query<{ p_id: number }>('P.all().filter(x => x.name == "alpha").collect()');
        assert.deepEqual(byName.map((row) => row.p_id), [2]);

        const byScore = await database.query<{ p_id: number }>("P.all().filter(x => x.score > 15).collect()");
        assert.deepEqual(byScore.map((row) => row.p_id).sort(), [1, 3]);

        // A non-boolean field path is a legal filter shape that matches nothing.
        const empty = await database.query("P.all().filter(x => x.score).collect()");
        assert.deepEqual(empty, []);
    });
});

test("text predicates compare Unicode code points without folding or normalizing", async () => {
    await seededDatabase(async (database) => {
        const contains = await database.query<{ p_id: number }>('P.all().filter(x => x.name.contains("elt")).collect()');
        assert.deepEqual(contains.map((row) => row.p_id), [1]);

        const starts = await database.query<{ p_id: number }>('P.all().filter(x => x.name.startsWith("ch")).collect()');
        assert.deepEqual(starts.map((row) => row.p_id), [3]);

        const ends = await database.query<{ p_id: number }>('P.all().filter(x => x.name.endsWith("ha")).collect()');
        assert.deepEqual(ends.map((row) => row.p_id), [2]);

        const cased = await database.query<{ p_id: number }>('P.all().filter(x => x.name.startsWith("Delta")).collect()');
        assert.deepEqual(cased, [], "matching is case sensitive");
    });
});

test("a text predicate on a non-text field is refused instead of matching nothing", async () => {
    await seededDatabase(async (database) => {
        const rejection = await rejectDiagnostic(() => database.query('P.all().filter(x => x.score.contains("3")).collect()'));
        assert.equal(rejection.code, "yydb.schema.invalid");
        assert.match(rejection.message, /requires a text field/);
        assert.match(rejection.message, /`score`/);
    });
});

test("ordering and windowing pipelines run over the seeded rows", async () => {
    await seededDatabase(async (database) => {
        const ascending = await database.query<{ p_id: number }>("P.all().sort_by(x => x.score).collect()");
        assert.deepEqual(ascending.map((row) => row.p_id), [2, 3, 1]);

        const descending = await database.query<{ p_id: number }>("P.all().sort_by_desc(x => x.score).collect()");
        assert.deepEqual(descending.map((row) => row.p_id), [1, 3, 2]);

        // Without an ordering step the engine walks rows in stored key order.
        const windowed = await database.query<{ p_id: number }>("P.all().skip(1).take(1).collect()");
        assert.deepEqual(windowed.map((row) => row.p_id), [2]);

        const limited = await database.query<{ p_id: number }>("P.all().take(2).collect()");
        assert.deepEqual(limited.map((row) => row.p_id), [1, 2]);
    });
});

test("projection selects fields through the `x => x.{ field }` shape", async () => {
    await seededDatabase(async (database) => {
        const rows = await database.query<{ name: string }>("P.all().sort_by(x => x.p_id).map(x => x.{ name }).collect()");
        assert.deepEqual(rows, [{ name: "delta" }, { name: "alpha" }, { name: "charlie" }]);
    });
});

test("unsupported pipeline spellings are refused with stable diagnostics", async () => {
    await seededDatabase(async (database) => {
        const rejections: ReadonlyArray<readonly [string, RegExp]> = [
            ["P.all().sortBy(x => x.score).collect()", /unsupported pipeline method/],
            ["P.all().filter(x => x.name.eq(\"alpha\")).collect()", /unsupported predicate method/],
            ["P.all().filter(x => !x.active).collect()", /field paths rooted at the lambda parameter/],
            ["P.all().filter(x => x.active && x.score.gt(15)).collect()", /unsupported predicate method/],
            ["P.all().map(x => x.{ label: name }).collect()", /field paths rooted at the lambda parameter/],
        ];
        for (const [source, pattern] of rejections) {
            const rejection = await rejectDiagnostic(() => database.query(source));
            assert.equal(rejection.code, "yydb.schema.invalid", source);
            assert.match(rejection.message, pattern, source);
        }
    });
});

test("aggregation and lookup terminals are not executable in Phase 1", async () => {
    await seededDatabase(async (database) => {
        for (const source of ["P.all().count()", "P.all().any()", "P.all().first()", "P.get(1)"]) {
            const rejection = await rejectDiagnostic(() => database.query(source));
            assert.equal(rejection.code, "yydb.schema.invalid", source);
            assert.match(rejection.message, /pipelines ending in `\.collect\(\)`/, source);
        }
    });
});

test("keyed update and delete programs patch one row and remove one row", async () => {
    await seededDatabase(async (database) => {
        await database.execute("P::update({ p_id: $id, active: false })", { id: 1 });
        await database.execute("P::delete({ p_id: $id })", { id: 2 });

        const rows = await database.query<{ p_id: number; name: string; score: number; active: boolean }>(
            "P.all().sort_by(x => x.p_id).collect()",
        );
        assert.deepEqual(
            rows.map((row) => row.p_id),
            [1, 3],
            "the keyed delete must remove exactly one row",
        );
        const patched = rows[0];
        assert.equal(patched.active, false, "the patched field must change");
        assert.equal(patched.name, "delta", "fields outside the patch must keep their stored value");
        assert.equal(patched.score, 30);
    });
});

test("collection update and delete programs stay unsupported", async () => {
    await seededDatabase(async (database) => {
        const rejection = await rejectDiagnostic(() =>
            database.execute("P.all().filter(x => x.p_id == 1).update(x => x.{ active: false })"),
        );
        assert.equal(rejection.code, "yydb.unsupported");
        assert.match(rejection.message, /table type/);
    });
});

test("rows written through programs survive a reopen of the same file", async () => {
    const directory = mkdtempSync(join(tmpdir(), "yydb-query-reopen-"));
    const path = join(directory, "reopen.yydb");
    try {
        const database = await Database.open(path);
        await database.ensureSchema(SEED_SCHEMA);
        for (const [id, name, score, active] of SEED_ROWS) {
            await database.execute("P::insert({ p_id: $id, name: $n, score: $s, active: $a })", { id, n: name, s: score, a: active });
        }
        await database.close();

        const reopened = await Database.open(path);
        try {
            const rows = await reopened.query<{ p_id: number; name: string; score: number; active: boolean }>(
                "P.all().sort_by(x => x.p_id).collect()",
            );
            assert.deepEqual(rows.map((row) => [row.p_id, row.name, row.score, row.active]), [
                [1, "delta", 30, true],
                [2, "alpha", 10, false],
                [3, "charlie", 20, true],
            ]);
        } finally {
            await reopened.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});
