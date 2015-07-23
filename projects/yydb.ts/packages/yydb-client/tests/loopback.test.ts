import assert from 'node:assert/strict';
import { spawn, type ChildProcess } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

import { connect, isYydbError, type Client } from '../dist/node.js';

/**
 * Real-serve loopback: spawn `yydb serve` on a private port and drive it with
 * the pure-TS client over TCP and WebSocket. The scripted-transport suites
 * cover frame shapes; this suite proves the client actually speaks the live
 * Hello → Authenticate handshake the server enforces.
 */

const HERE = dirname(fileURLToPath(import.meta.url));
const CLI_PATH = resolve(HERE, '..', '..', 'yydb', 'dist', 'cli.js');
const DEFAULT_TOKEN = 'yydb-phase0-default-token';

async function freePort(): Promise<number> {
    return await new Promise((resolvePort, rejectPort) => {
        const probe = createServer();
        probe.once('error', rejectPort);
        probe.listen(0, '127.0.0.1', () => {
            const address = probe.address();
            assert.ok(address && typeof address === 'object');
            const port = address.port;
            probe.close(() => resolvePort(port));
        });
    });
}

async function startServe(port: number, dbPath: string): Promise<ChildProcess> {
    const child = spawn(process.execPath, [CLI_PATH, 'serve', dbPath, '--bind', `127.0.0.1:${port}`], {
        stdio: 'ignore',
    });
    const deadline = Date.now() + 30_000;
    for (;;) {
        if (child.exitCode !== null) {
            throw new Error(`serve exited early with code ${child.exitCode}`);
        }
        try {
            const probe = await connect({ endpoint: `127.0.0.1:${port}` });
            probe.close();
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

async function withServe(run: (port: number) => Promise<void>): Promise<void> {
    const directory = mkdtempSync(join(tmpdir(), 'yydb-client-loopback-'));
    const port = await freePort();
    const child = await startServe(port, join(directory, 'loopback.yydb'));
    try {
        await run(port);
    } finally {
        await stopServe(child);
        for (let attempt = 0; attempt < 10; attempt += 1) {
            try {
                rmSync(directory, { recursive: true, force: true });
                return;
            } catch {
                await new Promise((wait) => setTimeout(wait, 100));
            }
        }
    }
}

const serveAvailable = existsSync(CLI_PATH);

test('the TCP client completes Hello plus Authenticate against a live serve', { skip: serveAvailable ? false : `missing ${CLI_PATH}` }, async () => {
    await withServe(async (port) => {
        const client: Client = await connect({ endpoint: `127.0.0.1:${port}`, token: DEFAULT_TOKEN });
        try {
            const payload = Uint8Array.from([0x00, 0xff, 0x10, 0x7f, 0x80, 0xde, 0xad, 0xbe, 0xef]);
            await client.put('loop/tcp', payload);
            const readBack = await client.get('loop/tcp');
            assert.ok(readBack);
            assert.deepEqual(Array.from(readBack), Array.from(payload));

            assert.equal(await client.get('loop/missing'), null);
            assert.match(await client.info(), /path=/);
        } finally {
            client.close();
        }
    });
});

test('the WebSocket client round-trips octet-exact values over /wire', { skip: serveAvailable ? false : `missing ${CLI_PATH}` }, async () => {
    await withServe(async (port) => {
        const client = await connect({ endpoint: `ws://127.0.0.1:${port}/wire`, token: DEFAULT_TOKEN });
        try {
            const text = 'yydb — 精确字节 ✓';
            await client.put('loop/ws', text);
            const readBack = await client.get('loop/ws');
            assert.ok(readBack);
            assert.equal(new TextDecoder().decode(readBack), text);
        } finally {
            client.close();
        }
    });
});

test('live serve rejects unauthenticated and wrong-token frames with the engine auth code', { skip: serveAvailable ? false : `missing ${CLI_PATH}` }, async () => {
    await withServe(async (port) => {
        const unauthenticated = await connect({ endpoint: `127.0.0.1:${port}` });
        try {
            await assert.rejects(unauthenticated.get('loop/any'), (error: unknown) => {
                assert.ok(isYydbError(error));
                assert.equal(error.code, 'yydb.auth.token_invalid');
                return true;
            });
        } finally {
            unauthenticated.close();
        }

        const wrongToken = connect({ endpoint: `127.0.0.1:${port}`, token: 'not-the-token' });
        await assert.rejects(wrongToken, (error: unknown) => {
            assert.ok(isYydbError(error));
            assert.equal(error.code, 'yydb.auth.token_invalid');
            return true;
        });
    });
});
