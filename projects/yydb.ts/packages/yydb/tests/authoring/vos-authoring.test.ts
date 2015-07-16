import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { Database, YydbError, v } from "@yydb/yydb";

/**
 * Author-side VOS surface: the `v` builder descriptors, the descriptor → `.vos`
 * rendering that `ensureSchema` performs, and the schema-document rejections the
 * engine enforces at publish time.
 */

async function withDatabase(run: (database: Database) => Promise<void>): Promise<void> {
    const directory = mkdtempSync(join(tmpdir(), "yydb-vos-authoring-"));
    try {
        const database = await Database.open(join(directory, "authoring.yydb"));
        try {
            await run(database);
        } finally {
            await database.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
}

/**
 * Native rejections arrive as plain `Error`s whose message is the diagnostic
 * JSON envelope; TS-minted rejections are already `YydbError`s. Normalize both.
 */
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

test("the `v` builder projects the frozen VOS type algebra onto descriptors", () => {
    const Owner = v.table("Owner", { owner_id: v.uuid().primary(), name: v.utf8() });
    const Post = v.table("Post", {
        post_id: v.i64().primary(),
        title: v.utf8().unique(),
        body: v.optional(v.utf8()),
        tags: v.list(v.utf8()),
        author: v.ref(Owner),
        thumb: v.file().optional(),
        emb: v.vector(4),
    });

    assert.equal(Post.kind, "table");
    assert.equal(Post.name, "Post");
    assert.deepEqual(Post.post_id.attrs, [{ name: "primary", args: [] }]);
    assert.deepEqual(Post.title.attrs, [{ name: "unique", args: [] }]);
    assert.equal(Post.body.kind, "optional");
    assert.equal(Post.body.inner?.kind, "utf8");
    assert.equal(Post.tags.kind, "list");
    assert.equal(Post.tags.inner?.kind, "utf8");
    assert.deepEqual(Post.author.target, { kind: "table", name: "Owner" });
    assert.equal(Post.thumb.kind, "optional");
    assert.equal(Post.thumb.inner?.kind, "file");
    assert.equal(Post.emb.kind, "vector");
    assert.equal(Post.emb.dim, 4);

    // Fields are addressable by name through `fields` and directly on the descriptor.
    assert.deepEqual(Object.keys(Post.fields), ["post_id", "title", "body", "tags", "author", "thumb", "emb"]);
    assert.equal(Post.post_id.name, "post_id");

    // Wrapper helpers append bracket attributes in application order.
    const flagged = v.utf8().primary().unique().attribute("tag", "7");
    assert.deepEqual(flagged.attrs.map((attr) => attr.name), ["primary", "unique", "tag"]);
    assert.deepEqual(flagged.attrs[2].args, ["7"]);
});

test("`ensureSchema` renders descriptors into the `.vos` document the engine stores", async () => {
    const Owner = v.table("Owner", { owner_id: v.uuid().primary(), name: v.utf8() });
    const Post = v.table("Post", {
        post_id: v.i64().primary(),
        title: v.utf8().unique(),
        tags: v.list(v.utf8()),
        author: v.ref(Owner),
        thumb: v.file().optional(),
        emb: v.vector(4),
    });
    const expected = [
        "table Owner {",
        "    @@owner_id: uuid,",
        "    name: utf8,",
        "}",
        "",
        "table Post {",
        "    @@post_id: i64,",
        "    @title: utf8,",
        "    tags: [utf8],",
        "    author: &Owner,",
        "    thumb: file?,",
        "    emb: vector<4>,",
        "}",
        "",
    ].join("\n");

    await withDatabase(async (database) => {
        assert.equal(await database.getSchema(), null, "a fresh database has no schema yet");
        await database.ensureSchema(Owner, Post);
        const schema = await database.getSchema();
        assert.ok(schema);
        assert.equal(schema.version, 1);
        assert.equal(schema.document, expected);
    });
});

test("`ensureSchema` accepts a raw `.vos` source string as well", async () => {
    await withDatabase(async (database) => {
        await database.ensureSchema("table User {\n    @@user_id: i64,\n    user_name: utf8,\n}\n");
        const schema = await database.getSchema();
        assert.ok(schema);
        assert.match(schema.document, /table User \{/);
        await database.execute('User::insert({ user_id: 1, user_name: "ada" })');
        const rows = await database.query<{ user_id: number; user_name: string }>("User.all().collect()");
        assert.deepEqual(rows, [{ user_id: 1, user_name: "ada" }]);
    });
});

test("re-publishing the same descriptor list is idempotent, a changed one is refused", async () => {
    const User = v.table("User", { user_id: v.i64().primary(), user_name: v.utf8() });
    await withDatabase(async (database) => {
        await database.ensureSchema(User);
        await database.ensureSchema(User);
        const unchanged = await database.getSchema();
        assert.ok(unchanged);
        assert.equal(unchanged.version, 1);

        const widened = v.table("User", { user_id: v.i64().primary(), user_name: v.utf8(), bio: v.optional(v.utf8()) });
        const drift = await rejectDiagnostic(() => database.ensureSchema(widened));
        assert.equal(drift.code, "yydb.schema.invalid");
        assert.match(drift.message, /schema document changed -- call migrate_schema/);
    });
});

test("every table needs exactly one primary field", async () => {
    await withDatabase(async (database) => {
        const missing = await rejectDiagnostic(() => database.ensureSchema("table X {\n    a: i64,\n}"));
        assert.equal(missing.code, "yydb.schema.invalid");
        assert.match(missing.message, /table `X` requires exactly one primary field/);

        const doubled = await rejectDiagnostic(() => database.ensureSchema("table Y {\n    @@a: i64,\n    @@b: i64,\n}"));
        assert.match(doubled.message, /table `Y` requires exactly one primary field/);
    });
});

test("non-VOS builtin names are refused at name resolution", async () => {
    await withDatabase(async (database) => {
        for (const [field, name] of [["s", "string"], ["j", "json"]] as const) {
            const rejection = await rejectDiagnostic(() =>
                database.ensureSchema(`table Bad_${field} {\n    @@x_id: i64,\n    ${field}: ${name},\n}`),
            );
            assert.equal(rejection.code, "yydb.schema.invalid");
            assert.match(rejection.message, /RES001/);
            assert.match(rejection.message, new RegExp(`unknown type ${name}`));
        }
    });
});

test("`vector` without a literal dimension is refused", async () => {
    await withDatabase(async (database) => {
        const rejection = await rejectDiagnostic(() =>
            database.ensureSchema("table Bad {\n    @@x_id: i64,\n    e: vector,\n}"),
        );
        assert.equal(rejection.code, "yydb.schema.invalid");
        assert.match(rejection.message, /RES019/);
        assert.match(rejection.message, /vector requires a literal dimension/);
    });
});

test("a named `class` declaration is accepted into the schema document", async () => {
    const Point = v.class("Point", { point_id: v.i64().primary(), x: v.f64(), y: v.f64() });
    await withDatabase(async (database) => {
        await database.ensureSchema(Point);
        const schema = await database.getSchema();
        assert.ok(schema);
        assert.match(schema.document, /class Point \{/);
    });
});

test("`migrateSchema` advances the version and rewrites the document with renames", async () => {
    const Item = v.table("Item", { item_id: v.i64().primary(), label: v.utf8() });
    const Renamed = v.table("Item", { item_id: v.i64().primary(), title: v.utf8() });
    await withDatabase(async (database) => {
        await database.ensureSchema(Item);
        await database.migrateSchema(Renamed, { renames: [{ type: "Item", fromField: "label", toField: "title" }] });
        const schema = await database.getSchema();
        assert.ok(schema);
        assert.equal(schema.version, 2);
        assert.match(schema.document, /title: utf8,/);
        assert.doesNotMatch(schema.document, /label/);
    });
});
