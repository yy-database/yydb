import assert from 'node:assert/strict';
import { spawn, type ChildProcess } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

import {
    DatabaseError,
    Yydb,
    YydbClient,
    isDatabaseError,
    v,
    type Statement,
} from '@yydb/yydb';

/**
 * Author-side surface of the formal constructors: `new Yydb()` / `new
 * YydbClient()`, the descriptor-derived query builder, the DDL session, the
 * error envelope and the capability reports. Everything runs against the
 * real engine (N-API) or a real `yydb serve` loopback — no mocks.
 */

const User = v.table('User', { user_id: v.i64().primary(), user_name: v.utf8() });
const UserRenamed = v.table('User', { user_id: v.i64().primary(), display_name: v.utf8() });

async function withYydb(run: (db: Yydb) => Promise<void>): Promise<void> {
    const directory = mkdtempSync(join(tmpdir(), 'yydb-ctor-'));
    try {
        const db = new Yydb({ storage: { filename: join(directory, 'authoring.yydb') } });
        await db.open();
        try {
            await run(db);
        } finally {
            await db.close();
        }
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
}

async function seedUsers(db: Yydb): Promise<void> {
    await db.ensureSchema([User]);
    await db.table(User).insert({ user_id: 1, user_name: 'ada' });
    await db.table(User).insert({ user_id: 2, user_name: 'grace' });
    await db.table(User).insert({ user_id: 3, user_name: 'alan' });
}

async function rejection(run: () => Promise<unknown>): Promise<DatabaseError> {
    let caught: unknown;
    try {
        await run();
    } catch (cause) {
        caught = cause;
    }
    assert.ok(caught, 'expected the call to reject');
    assert.ok(isDatabaseError(caught), `expected a DatabaseError envelope, got ${String(caught)}`);
    return caught;
}

test('the constructor only validates configuration; business calls require an open engine', async () => {
    const directory = mkdtempSync(join(tmpdir(), 'yydb-ctor-'));
    try {
        const db = new Yydb({ storage: { filename: join(directory, 'lifecycle.yydb') } });
        assert.equal(db.lifecycle, 'new');
        const early = await rejection(() => Promise.resolve(db.table(User)));
        assert.equal(early.code, 'yydb.state.invalid');
        assert.equal(early.phase, 'execute');
        assert.equal(early.execution, 'not-sent');
        assert.equal(early.product, 'yydb');

        await db.close();
        assert.equal(db.lifecycle, 'closed');
        await db.close(); // repeated close succeeds
        const revive = await rejection(() => db.open());
        assert.equal(revive.code, 'yydb.state.invalid');

        assert.throws(() => new Yydb({ storage: { filename: '' } }), (error: unknown) => {
            assert.ok(isDatabaseError(error));
            assert.equal(error.phase, 'configuration');
            return true;
        });
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test('open rejects the options the native binding cannot honour', async () => {
    const directory = mkdtempSync(join(tmpdir(), 'yydb-ctor-'));
    try {
        const readOnly = new Yydb({ storage: { filename: join(directory, 'ro.yydb') }, readOnly: true });
        const ro = await rejection(() => readOnly.open());
        assert.equal(ro.code, 'yydb.unsupported');
        assert.equal(ro.phase, 'open');
        assert.equal(readOnly.lifecycle, 'failed');

        const inMemory = new Yydb();
        const mem = await rejection(() => inMemory.open());
        assert.equal(mem.code, 'yydb.unsupported');
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

test('the table builder projects the VOS operators onto the native pipeline', async () => {
    await withYydb(async (db) => {
        await seedUsers(db);

        const all = await db.table(User).collect();
        assert.equal(all.length, 3);
        assert.deepEqual(all.map((row) => row.user_name).sort(), ['ada', 'alan', 'grace']);

        const filtered = await db.table(User).filter('row => row.user_id > 1').collect();
        assert.equal(filtered.length, 2);

        const projected = await db.table(User).map('row => row.{ user_name }').collect();
        assert.deepEqual(Object.keys(projected[0]!), ['user_name']);
        const aliased = await db.table(User).map('row => row.{ name: row.user_name }').collect();
        assert.deepEqual(Object.keys(aliased[0]!), ['name']);

        const asc = await db.table(User).sort_by(User.user_id).collect();
        assert.deepEqual(asc.map((row) => row.user_id), [1, 2, 3]);
        const desc = await db.table(User).sort_by_desc('user_id').collect();
        assert.deepEqual(desc.map((row) => row.user_id), [3, 2, 1]);

        const page = await db.table(User).sort_by(User.user_id).skip(1).take(1).collect();
        assert.deepEqual(page.map((row) => row.user_id), [2]);

        const bound = await db.table(User).filter('row => row.user_name == $who').collect({ who: 'ada' });
        assert.deepEqual(bound.map((row) => row.user_id), [1]);

        assert.deepEqual((await db.table(User).get(2))?.user_name, 'grace');
        assert.equal(await db.table(User).get(99), null);
        assert.equal((await db.table(User).sort_by(User.user_id).first())?.user_id, 1);
        assert.equal(await db.table(User).filter('row => row.user_id > 3').any(), false);
        assert.equal(await db.table(User).count(), 3);
        assert.equal(await db.table(User).filter('row => row.user_id > 1').count(), 2);
    });
});

test('builder configuration mistakes fail as envelope errors before the engine is consulted', async () => {
    await withYydb(async (db) => {
        await seedUsers(db);
        const badSort = await rejection(() => Promise.resolve(db.table(User).sort_by('missing_field')));
        assert.equal(badSort.code, 'yydb.configuration.invalid');
        assert.equal(badSort.execution, 'not-sent');

        const badWindow = await rejection(() => db.table(User).take(1.5).collect());
        assert.equal(badWindow.code, 'yydb.configuration.invalid');
    });
});

test('prepared statements run through the builder and die with the instance', async () => {
    await withYydb(async (db) => {
        await seedUsers(db);
        const statement: Statement = await db.prepare('User.all().take($limit).collect()');
        assert.equal((await statement.execute({ limit: 2 })).length, 2);
        await db.close();
        assert.equal(statement.isClosed, true, 'close() must cascade into tracked statements');
    });
});

test('engine diagnostics land in the family envelope with product/phase/execution/retry', async () => {
    await withYydb(async (db) => {
        await db.ensureSchema([User]);
        const drift = await rejection(() => db.ensureSchema([UserRenamed]));
        assert.equal(drift.product, 'yydb');
        assert.equal(drift.code, 'yydb.schema.invalid');
        assert.equal(drift.phase, 'execute');
        assert.equal(drift.execution, 'replied');
        assert.equal(drift.retry, 'never');
        assert.match(drift.message, /migrate_schema/);
    });
});

test('transactions commit on return, roll back on throw and pass user errors through', async () => {
    await withYydb(async (db) => {
        await seedUsers(db);
        await db.transaction(async (tx) => {
            await tx.execute('User::insert({ user_id: 9, user_name: "nine" })');
        });
        assert.equal(await db.table(User).get(9) !== null, true);

        const marker = new RangeError('user callback failure');
        await assert.rejects(
            db.transaction(async (tx) => {
                await tx.execute('User::insert({ user_id: 10, user_name: "ten" })');
                throw marker;
            }),
            (error) => error === marker,
        );
        assert.equal(await db.table(User).get(10), null);
    });
});

test('the DDL session queues renames, commits through the native migration and invalidates plans', async () => {
    await withYydb(async (db) => {
        await seedUsers(db);
        const statement = await db.prepare('User.all().collect()');

        const ddl = await db.beginDdlSession({ ttlMs: 30_000 });
        ddl.renameField('User', 'user_name', 'display_name');
        await ddl.commit(UserRenamed);

        assert.equal(statement.isClosed, true, 'a committed DDL revision must invalidate prepared plans');
        const schema = await db.getSchema();
        assert.equal(schema?.version, 2);
        assert.match(schema!.document, /display_name: utf8,/);

        const late = await rejection(() => Promise.resolve(ddl.renameField('User', 'a', 'b')));
        assert.equal(late.code, 'yydb.ddl.closed');

        await db.table(UserRenamed).insert({ user_id: 4, display_name: 'd' });
        const rows = await db.table(UserRenamed).collect();
        assert.equal(rows.length, 4);
        const fresh = rows.find((row) => (row as { user_id: number }).user_id === 4)!;
        assert.deepEqual(Object.keys(fresh).sort(), ['display_name', 'user_id']);
        const legacy = rows.find((row) => (row as { user_id: number }).user_id === 1)!;
        assert.equal('user_name' in legacy, true, 'the rename preserves record identity without rewriting stored keys');
    });
});

test('an expired DDL session refuses further operations', async () => {
    await withYydb(async (db) => {
        await db.ensureSchema([User]);
        const ddl = await db.beginDdlSession({ ttlMs: 1 });
        await new Promise((sleep) => setTimeout(sleep, 10));
        assert.equal(ddl.isExpired, true);
        const expired = await rejection(() => Promise.resolve(ddl.renameField('User', 'user_name', 'x')));
        assert.equal(expired.code, 'yydb.ddl.expired');
        await ddl.close();
    });
});

test('listen reports the missing non-blocking serve primitive instead of faking a listener', async () => {
    await withYydb(async (db) => {
        const gap = await rejection(() => db.listen());
        assert.equal(gap.code, 'yydb.unsupported');
        assert.equal(gap.phase, 'open');
        assert.equal(db.capabilities().listen, false);
    });
});

test('capabilities is a stable per-deployment structure', async () => {
    await withYydb(async (db) => {
        const caps = db.capabilities();
        assert.equal(caps.product, 'yydb');
        assert.equal(caps.deployment, 'local');
        assert.equal(typeof caps.engineVersion, 'string');
        assert.equal(caps.readOnlyOpen, false);
        assert.equal(caps.storage.inMemory, false);
        assert.equal(caps.query.dialect, 'vos');
        assert.ok(caps.query.pipeline.includes('sort_by_desc'));
        assert.equal(caps.query.terminals.count, 'composed');
        assert.equal(caps.transactions.nested, false);
        assert.equal(caps.ddl.session, 'rename-queue');
    });
});

test('YydbClient validates the address union and refuses options without wire primitives', () => {
    assert.throws(() => new YydbClient({ url: 'ws://127.0.0.1:7700/wire', endpoint: { host: '127.0.0.1', port: 7700 } }), (error: unknown) => {
        assert.ok(isDatabaseError(error));
        assert.equal(error.code, 'yydb.client.invalid_endpoint');
        assert.equal(error.phase, 'configuration');
        return true;
    });
    assert.throws(() => new YydbClient({ url: 'yydb://127.0.0.1:7700' }), (error: unknown) => {
        assert.ok(isDatabaseError(error));
        assert.equal(error.code, 'yydb.client.invalid_endpoint');
        return true;
    });
    assert.throws(() => new YydbClient({ endpoint: { host: '127.0.0.1', port: 0 } }), (error: unknown) => {
        assert.ok(isDatabaseError(error));
        return true;
    });
    for (const extra of [{ tls: { serverName: 'x' } }, { pool: { max: 4 } }, { connectTimeoutMs: 500 }]) {
        assert.throws(
            () => new YydbClient({ endpoint: { host: '127.0.0.1', port: 7700 }, ...extra }),
            (error: unknown) => isDatabaseError(error) && error.code === 'yydb.unsupported',
        );
    }
});

test('YydbClient without an address refuses to connect and pre-connect calls fail on state', async () => {
    const client = new YydbClient({} as never);
    const missing = await rejection(() => client.connect());
    assert.equal(missing.code, 'yydb.client.invalid_endpoint');
    assert.equal(missing.phase, 'connect');

    const notReady = await rejection(() => Promise.resolve(client.table(User)));
    assert.equal(notReady.code, 'yydb.state.invalid');
    const gap = await rejection(() => client.prepare('User.all().collect()'));
    assert.equal(gap.code, 'yydb.state.invalid');
    assert.equal(client.capabilities().deployment, 'remote');
    assert.equal(client.capabilities().query, false);
});

// ─── Live `yydb serve` loopback ────────────────────────────────────────────

const HERE = dirname(fileURLToPath(import.meta.url));
const CLI_PATH = resolve(HERE, '..', '..', 'dist', 'cli.js');
const DEFAULT_TOKEN = 'yydb-phase0-default-token';

async function freePort(): Promise<number> {
    return await new Promise((resolvePort, rejectPort) => {
        const probe = createServer();
        probe.once('error', rejectPort);
        probe.listen(0, '127.0.0.1', () => {
            const address = probe.address();
            assert.ok(address && typeof address === 'object');
            probe.close(() => resolvePort(address.port));
        });
    });
}

async function startServe(port: number, dbPath: string): Promise<ChildProcess> {
    const child = spawn(process.execPath, [CLI_PATH, 'serve', dbPath, '--bind', `127.0.0.1:${port}`], { stdio: 'ignore' });
    const deadline = Date.now() + 30_000;
    for (;;) {
        if (child.exitCode !== null) {
            throw new Error(`serve exited early with code ${child.exitCode}`);
        }
        try {
            const probe = new YydbClient({ endpoint: { host: '127.0.0.1', port }, auth: { token: DEFAULT_TOKEN } });
            await probe.connect();
            await probe.close();
            return child;
        } catch {
            if (Date.now() > deadline) {
                child.kill();
                throw new Error('serve did not become ready within 30s');
            }
            await new Promise((wait) => setTimeout(wait, 100));
        }
    }
}

function stopServe(child: ChildProcess): Promise<void> {
    return new Promise((resolveExit) => {
        child.once('exit', () => resolveExit());
        child.kill();
    });
}

const serveAvailable = existsSync(CLI_PATH);

test('YydbClient drives a live serve over TCP: auth, KV, schema, and honest VOS gaps', { skip: serveAvailable ? false : `missing ${CLI_PATH}` }, async () => {
    const directory = mkdtempSync(join(tmpdir(), 'yydb-client-loop-'));
    const port = await freePort();
    const child = await startServe(port, join(directory, 'loop.yydb'));
    try {
        const client = new YydbClient({ endpoint: { host: '127.0.0.1', port }, auth: { token: DEFAULT_TOKEN } });
        await client.connect();
        assert.equal(client.lifecycle, 'ready');
        assert.match(await client.serverVersion(), /\d+/);
        assert.equal(client.capabilities().serverVersion !== null, true);

        await client.put('ctor/key', 'value');
        assert.deepEqual([...(await client.get('ctor/key'))!], [...new TextEncoder().encode('value')]);
        assert.equal(await client.get('ctor/absent'), null);

        const Note = v.table('Note', { note_id: v.i64().primary(), text: v.utf8() });
        await client.ensureSchema([Note]);
        const schema = await client.getSchema();
        assert.match(schema!.document, /table Note \{/);

        const tableGap = await rejection(() => Promise.resolve(client.table(Note)));
        assert.equal(tableGap.code, 'yydb.unsupported');
        const prepareGap = await rejection(() => client.prepare('Note.all().collect()'));
        assert.equal(prepareGap.code, 'yydb.unsupported');
        const txGap = await rejection(() => client.transaction(async () => 1));
        assert.equal(txGap.code, 'yydb.unsupported');

        await client.close();
        await client.close();
        assert.equal(client.lifecycle, 'closed');
        const afterClose = await rejection(() => client.get('ctor/key'));
        assert.equal(afterClose.code, 'yydb.state.closed');
    } finally {
        await stopServe(child);
        rmSync(directory, { recursive: true, force: true });
    }
});

test('YydbClient reaches the same serve over the ws:// url form', { skip: serveAvailable ? false : `missing ${CLI_PATH}` }, async () => {
    const directory = mkdtempSync(join(tmpdir(), 'yydb-client-ws-'));
    const port = await freePort();
    const child = await startServe(port, join(directory, 'ws.yydb'));
    try {
        const client = new YydbClient({ url: `ws://127.0.0.1:${port}/wire`, auth: { token: DEFAULT_TOKEN } });
        await client.connect();
        await client.put('ctor/ws', 'wired');
        assert.equal(new TextDecoder().decode((await client.get('ctor/ws'))!), 'wired');
        await client.close();
    } finally {
        await stopServe(child);
        rmSync(directory, { recursive: true, force: true });
    }
});

test('YydbClient surfaces a wrong token as an authenticated-phase envelope failure', { skip: serveAvailable ? false : `missing ${CLI_PATH}` }, async () => {
    const directory = mkdtempSync(join(tmpdir(), 'yydb-client-auth-'));
    const port = await freePort();
    const child = await startServe(port, join(directory, 'auth.yydb'));
    try {
        const client = new YydbClient({ endpoint: { host: '127.0.0.1', port }, auth: { token: 'not-the-token' } });
        const failure = await rejection(() => client.connect());
        assert.equal(failure.phase, 'connect');
        assert.equal(failure.execution, 'replied');
        assert.match(failure.code, /^yydb\./);
        assert.equal(client.lifecycle, 'failed');
    } finally {
        await stopServe(child);
        rmSync(directory, { recursive: true, force: true });
    }
});
