import { spawn, type ChildProcess } from 'node:child_process';
import fs from 'node:fs';
import net from 'node:net';
import path from 'node:path';
import { connect, type Client } from '@yydb/yydb-client/node';
import { defineMicro as buildMicro, microFingerprint, type DefinedMicro, type MicroHandle, MicroSessionRegistry } from '../shared/micro.js';
import { resolveYydbCli } from './resolve-bin.js';
import type { UdfScalarKind, UdfTypeDescriptor } from '../shared/udf-types.js';

export type { SchemaVersion } from '@yydb/yydb-client';

export interface OpenOptions {
    /** Override CLI script path (tests / special installs). */
    cli?: string;
    /** @deprecated Use `cli`. */
    binary?: string;
    /** Prefer a specific loopback port; default = ephemeral free port. */
    port?: number;
    /** Extra args after `serve <path> --bind …`. */
    serveArgs?: string[];
}

async function freeLoopbackPort(): Promise<number> {
    return await new Promise((resolve, reject) => {
        const server = net.createServer();
        server.listen(0, '127.0.0.1', () => {
            const address = server.address();
            if (!address || typeof address === 'string') {
                server.close();
                reject(new Error('could not allocate loopback port'));
                return;
            }
            const { port } = address;
            server.close((error) => {
                if (error) reject(error);
                else resolve(port);
            });
        });
        server.on('error', reject);
    });
}

async function waitForTcp(host: string, port: number, timeoutMs: number) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        try {
            await new Promise<void>((resolve, reject) => {
                const socket = net.connect({ host, port }, () => {
                    socket.end();
                    resolve();
                });
                socket.on('error', reject);
            });
            return;
        } catch {
            await new Promise((r) => setTimeout(r, 40));
        }
    }
    throw new Error(`yydb engine did not accept connections on ${host}:${port}`);
}

/**
 * Node-facing YYDB handle. Looks like one library; owns a private loopback
 * `yydb serve` child under the hood.
 */
export class Database {
    readonly path: string;
    readonly endpoint: string;
    private client: Client;
    private child: ChildProcess | null;
    private readonly microRegistry: MicroSessionRegistry;

    private constructor(dbPath: string, endpoint: string, client: Client, child: ChildProcess) {
        this.path = dbPath;
        this.endpoint = endpoint;
        this.client = client;
        this.child = child;
        this.microRegistry = new MicroSessionRegistry();
        this.client.setMicroHostHandler((payload) => this.microRegistry.invokeFromWire(payload));
    }

    /**
     * Open (create) a `.yydb` file. Spawns the engine on `127.0.0.1` automatically.
     */
    static async open(dbPath: string, options: OpenOptions = {}): Promise<Database> {
        const resolved = path.resolve(dbPath);
        fs.mkdirSync(path.dirname(resolved), { recursive: true });

        const cli = options.cli ?? options.binary ?? resolveYydbCli();
        const port = options.port ?? (await freeLoopbackPort());
        const bind = `127.0.0.1:${port}`;
        const args = [cli, 'serve', resolved, '--bind', bind, ...(options.serveArgs ?? [])];

        const child = spawn(process.execPath, args, {
            stdio: 'pipe',
            windowsHide: true,
        });

        let stderr = '';
        child.stderr?.setEncoding('utf8');
        child.stderr?.on('data', (chunk: string) => {
            stderr += chunk;
        });

        const exitPromise = new Promise<never>((_, reject) => {
            child.once('exit', (code, signal) => {
                reject(new Error(`yydb engine exited early (code=${code}, signal=${signal}): ${stderr.trim()}`));
            });
            child.once('error', reject);
        });

        try {
            await Promise.race([waitForTcp('127.0.0.1', port, 8_000), exitPromise]);
            const client = await connect(bind);
            child.removeAllListeners('exit');
            child.removeAllListeners('error');
            child.on('exit', () => {
                /* closed via Database.close or crash */
            });
            return new Database(resolved, bind, client, child);
        } catch (error) {
            child.kill('SIGTERM');
            throw error;
        }
    }

    async serverVersion(): Promise<string> {
        return this.client.serverVersion();
    }

    async info(): Promise<string> {
        return this.client.info();
    }

    async getSchema() {
        return this.client.getSchema();
    }

    async ensureSchema(version: number, document: string): Promise<void> {
        await this.client.ensureSchema(version, document);
    }

    async get(key: string): Promise<Uint8Array | null> {
        return this.client.get(key);
    }

    async put(key: string, value: Uint8Array | string): Promise<void> {
        await this.client.put(key, value);
    }

    /**
     * Build a typed session-local micro definition. Does not register it with
     * the engine until [`registerMicro`](./database.ts) is called.
     */
    defineMicro<TArgs extends readonly UdfTypeDescriptor[], TReturn extends UdfTypeDescriptor>(
        definition: Parameters<typeof buildMicro<TArgs, TReturn>>[0],
    ): DefinedMicro<TArgs, TReturn> {
        return buildMicro(definition);
    }

    /**
     * Install a [`defineMicro`](./database.ts) definition into the current
     * session registry and sync metadata to the private engine. Host micros are
     * session-local and never written to `.yydb`.
     */
    async registerMicro<TArgs extends readonly UdfTypeDescriptor[], TReturn extends UdfTypeDescriptor>(
        definition: DefinedMicro<TArgs, TReturn>,
    ): Promise<MicroHandle> {
        const handle = this.microRegistry.register(definition);
        await this.client.registerMicro({
            hostId: handle.hostId,
            handleVersion: handle.implementationVersion,
            udfVersion: definition.version,
            name: definition.name,
            functionId: handle.functionId,
            args: definition.args.map((arg) => arg.kind as UdfScalarKind),
            returns: definition.returns.kind as UdfScalarKind,
            fingerprint: microFingerprint(definition.name, definition.version),
        });
        return handle;
    }

    /** Invoke a registered scalar UDF on the private engine. */
    async callScalar(
        name: string,
        version: number,
        args: readonly (string | number | boolean | null)[],
    ): Promise<string | number | boolean | null> {
        return this.client.callScalar(name, version, args);
    }

    /** Stop the private engine process. */
    async close(): Promise<void> {
        this.client.close();
        const child = this.child;
        this.child = null;
        if (!child || child.exitCode !== null || child.killed) {
            return;
        }
        await new Promise<void>((resolve) => {
            child.once('exit', () => resolve());
            child.kill('SIGTERM');
            setTimeout(() => {
                if (child.exitCode === null && !child.killed) {
                    child.kill('SIGKILL');
                }
                resolve();
            }, 2_000);
        });
    }
}
