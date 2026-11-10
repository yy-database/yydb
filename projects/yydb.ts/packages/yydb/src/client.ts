/**
 * Formal remote constructor for YYDB.
 *
 * `YydbClient` owns a connection to an existing `yydb serve` endpoint and
 * projects the same VOS data-author surface as {@link Yydb}. It never starts,
 * repairs or manages a server, and the client path initialises no local
 * engine: every call delegates to the maintained YY wire protocol primitives
 * in `@yydb/yydb-client` (Hello → Authenticate → frames).
 *
 * Surface honesty: the wire currently exposes KV, schema and micro frames.
 * The VOS query terminals (`table` / `prepare` / `transaction`) have no
 * client-side wire primitive yet — the server answers Prepare/ExecutePrepared
 * (frame codes 21–24) but neither the wire client nor the N-API `Client`
 * speaks them — so those methods fail with `yydb.unsupported` and
 * {@link YydbClient.capabilities} reports them as unavailable instead of
 * faking them in TypeScript.
 */

import { emitSource, type EmittableItem, type TableDescriptor } from '@voml/vos';
import { connect as connectWire, type Client as WireClient } from '@yydb/yydb-client/node';
import { DatabaseError } from './shared/envelope.js';
import type { TableQuery } from './yydb.js';

/** Lifecycle state of a {@link YydbClient}. Terminal states never revive. */
export type YydbClientState = 'new' | 'connecting' | 'ready' | 'closing' | 'closed' | 'failed';

/** Structured TCP address. */
export interface TcpEndpoint {
    host: string;
    port: number;
}

/** URL and structured endpoint are mutually exclusive; exactly one selects the transport. */
export type YydbClientAddress =
    | { url: string; endpoint?: never }
    | { url?: never; endpoint: TcpEndpoint };

/** Bearer-token authentication sent in the `Authenticate` frame after Hello. */
export interface YydbAuth {
    token: string;
}

/** TLS material. The YY wire has no TLS primitive yet; supplying it is a configuration error. */
export interface YydbTlsOptions {
    ca?: readonly Uint8Array[];
    serverName?: string;
    identity?: { certificate: Uint8Array; privateKey: Uint8Array };
}

/** Connection-pool sizing. The wire session is a single connection; supplying a pool is a configuration error. */
export interface YydbPoolOptions {
    min?: number;
    max?: number;
    maxPending?: number;
    acquireTimeoutMs?: number;
    idleTimeoutMs?: number;
}

/** Options accepted by `new YydbClient(options)`. */
export type YydbClientOptions = YydbClientAddress & {
    auth?: YydbAuth;
    tls?: YydbTlsOptions;
    pool?: YydbPoolOptions;
    connectTimeoutMs?: number;
};

/** Stable capability structure for {@link YydbClient.capabilities}. */
export interface YydbClientCapabilities {
    readonly product: 'yydb';
    readonly deployment: 'remote';
    readonly wireVersion: '0000';
    readonly serverVersion: string | null;
    readonly kv: { readonly get: boolean; readonly put: boolean };
    readonly schema: { readonly ensure: boolean; readonly get: boolean; readonly migrate: boolean };
    readonly micro: { readonly register: boolean; readonly scalarCall: boolean };
    readonly query: false;
    readonly prepared: false;
    readonly transactions: false;
    readonly tls: false;
    readonly pool: false;
    readonly maxFrameBodyBytes: number;
}

/**
 * Remote YYDB connection owner.
 *
 * ```ts
 * const client = new YydbClient({ endpoint: { host: '127.0.0.1', port: 7700 }, auth: { token } });
 * await client.connect();
 * await client.put('a', '1');
 * const value = await client.get('a');
 * await client.close();
 * ```
 */
export class YydbClient {
    private state: YydbClientState = 'new';
    private connecting: Promise<void> | null = null;
    private wire: WireClient | null = null;
    private cachedVersion: string | null = null;
    private readonly target: string;
    private readonly token: string | undefined;

    /**
     * Save and validate configuration only. `url` and `endpoint` are mutually
     * exclusive; TLS, pool and connect-timeout have no wire primitive and are
     * refused here rather than silently dropped.
     */
    constructor(options: YydbClientOptions) {
        if (options === null || typeof options !== 'object') {
            throw new DatabaseError('yydb.configuration.invalid', 'YydbClient options must be an object', {
                phase: 'configuration',
                execution: 'not-sent',
            });
        }
        const hasUrl = options.url !== undefined;
        const hasEndpoint = options.endpoint !== undefined;
        if (hasUrl && hasEndpoint) {
            throw new DatabaseError('yydb.client.invalid_endpoint', 'options.url and options.endpoint are mutually exclusive', {
                phase: 'configuration',
                execution: 'not-sent',
            });
        }
        if (hasUrl) {
            if (typeof options.url !== 'string' || !(options.url.startsWith('ws://') || options.url.startsWith('wss://'))) {
                throw new DatabaseError(
                    'yydb.client.invalid_endpoint',
                    'options.url must be a ws:// or wss:// serve endpoint; use options.endpoint for TCP',
                    { phase: 'configuration', execution: 'not-sent' },
                );
            }
            this.target = options.url;
        } else if (hasEndpoint) {
            const { host, port } = options.endpoint;
            if (typeof host !== 'string' || !host || !Number.isInteger(port) || port < 1 || port > 65_535) {
                throw new DatabaseError('yydb.client.invalid_endpoint', 'options.endpoint requires a non-empty host and a valid port', {
                    phase: 'configuration',
                    execution: 'not-sent',
                });
            }
            this.target = `${host}:${port}`;
        } else {
            this.target = '';
        }
        if (options.auth !== undefined) {
            if (typeof options.auth.token !== 'string' || !options.auth.token) {
                throw new DatabaseError('yydb.configuration.invalid', 'options.auth.token must be a non-empty string', {
                    phase: 'configuration',
                    execution: 'not-sent',
                });
            }
            this.token = options.auth.token;
        }
        for (const [label, value] of [['tls', options.tls], ['pool', options.pool], ['connectTimeoutMs', options.connectTimeoutMs]] as const) {
            if (value !== undefined) {
                throw new DatabaseError(
                    'yydb.unsupported',
                    `options.${label} has no YY wire primitive; accepting it would silently drop the requested guarantee`,
                    { phase: 'configuration', execution: 'not-sent' },
                );
            }
        }
    }

    /** Current lifecycle state. */
    get lifecycle(): YydbClientState {
        return this.state;
    }

    /**
     * Open the wire session: transport, Hello and — when `auth` was supplied —
     * Authenticate. Concurrent calls merge into one operation.
     */
    async connect(): Promise<this> {
        if (this.state === 'ready') {
            return this;
        }
        if (this.connecting) {
            return this.connecting.then(() => this);
        }
        if (this.state !== 'new') {
            throw new DatabaseError('yydb.state.invalid', `cannot connect a ${this.state} YydbClient — construct a new instance`, {
                phase: 'connect',
                execution: 'not-sent',
            });
        }
        if (!this.target) {
            throw new DatabaseError(
                'yydb.client.invalid_endpoint',
                'no serve address configured — pass options.url (ws://…) or options.endpoint ({ host, port })',
                { phase: 'connect', execution: 'not-sent' },
            );
        }
        this.state = 'connecting';
        this.connecting = (async () => {
            try {
                this.wire = await connectWire({ endpoint: this.target, token: this.token });
                this.cachedVersion = await this.wire.serverVersion();
                this.state = 'ready';
            } catch (cause) {
                this.state = 'failed';
                const transportFailure = isCode(cause, 'yydb.client.transport_failure') || isCode(cause, 'yydb.client.invalid_endpoint');
                throw DatabaseError.from(cause, {
                    phase: 'connect',
                    execution: transportFailure ? 'not-sent' : 'replied',
                });
            } finally {
                this.connecting = null;
            }
        })();
        return this.connecting.then(() => this);
    }

    /** Server version string (re-issues Hello on the wire). */
    async serverVersion(): Promise<string> {
        const wire = this.requireReady('serverVersion');
        return guardWire(() => wire.serverVersion());
    }

    /** Remote info text (`path=…`). */
    async info(): Promise<string> {
        const wire = this.requireReady('info');
        return guardWire(() => wire.info());
    }

    /** Published schema metadata on the remote engine, or `null` before the first schema. */
    async getSchema(): Promise<{ version: number; document: string } | null> {
        const wire = this.requireReady('getSchema');
        return guardWire(() => wire.getSchema());
    }

    /** Validate and publish a VOS schema document on the remote engine — never an implicit migration. */
    async ensureSchema(tables: readonly EmittableItem[]): Promise<void> {
        const wire = this.requireReady('ensureSchema');
        if (!Array.isArray(tables) || tables.length === 0) {
            throw new DatabaseError('yydb.configuration.invalid', 'ensureSchema requires at least one descriptor', {
                phase: 'configuration',
                execution: 'not-sent',
            });
        }
        const document = emitSource([...tables]);
        await guardWire(() => wire.ensureSchema(0, document));
    }

    /** Read one remote record, or `null` when the key is absent. */
    async get(key: string): Promise<Uint8Array | null> {
        const wire = this.requireReady('get');
        return guardWire(() => wire.get(key));
    }

    /** Write one remote record. */
    async put(key: string, value: Uint8Array | string): Promise<void> {
        const wire = this.requireReady('put');
        return guardWire(() => wire.put(key, value));
    }

    /**
     * Query builder over a VOS descriptor. The wire client has no
     * Prepare/ExecutePrepared frames yet, so this fails with
     * `yydb.unsupported`; the descriptor shape is still validated here.
     */
    table<T extends TableDescriptor>(descriptor: T): TableQuery<T> {
        this.requireReady('table');
        if (descriptor === null || typeof descriptor !== 'object' || descriptor.kind !== 'table' || typeof descriptor.name !== 'string') {
            throw new DatabaseError('yydb.configuration.invalid', 'table() requires a `v.table()` descriptor', {
                phase: 'configuration',
                execution: 'not-sent',
            });
        }
        throw remoteVosGap('table()');
    }

    /** Prepare a VOS pipeline remotely — refused until the wire exposes query frames (codes 21–24). */
    async prepare(source: string): Promise<never> {
        this.requireReady('prepare');
        if (typeof source !== 'string' || !source.trim()) {
            throw new DatabaseError('yydb.configuration.invalid', 'prepare() requires VOS source text', {
                phase: 'configuration',
                execution: 'not-sent',
            });
        }
        throw remoteVosGap('prepare()');
    }

    /** Run a transaction remotely — refused; the wire has no transaction frames. */
    async transaction<T>(fn: (tx: unknown) => Promise<T>): Promise<T> {
        this.requireReady('transaction');
        if (typeof fn !== 'function') {
            throw new DatabaseError('yydb.configuration.invalid', 'transaction() requires a callback', {
                phase: 'configuration',
                execution: 'not-sent',
            });
        }
        throw remoteVosGap('transaction()');
    }

    /** Stable capability structure for this remote deployment. */
    capabilities(): YydbClientCapabilities {
        return {
            product: 'yydb',
            deployment: 'remote',
            wireVersion: '0000',
            serverVersion: this.state === 'ready' ? this.cachedVersion : null,
            kv: { get: true, put: true },
            schema: { ensure: true, get: true, migrate: false },
            micro: { register: true, scalarCall: true },
            query: false,
            prepared: false,
            transactions: false,
            tls: false,
            pool: false,
            maxFrameBodyBytes: 16 * 1024 * 1024,
        };
    }

    /**
     * Close the connection pool and everything derived from it. The remote
     * database keeps running; repeated closes succeed.
     */
    async close(): Promise<void> {
        if (this.state === 'closed') {
            return;
        }
        if (this.state === 'new') {
            this.state = 'closed';
            return;
        }
        this.state = 'closing';
        try {
            if (this.connecting) {
                try {
                    await this.connecting;
                } catch {
                    // connect already failed; the cascade below just settles state
                }
            }
            this.wire?.close();
            this.wire = null;
            this.state = 'closed';
        } catch (cause) {
            this.state = 'failed';
            throw DatabaseError.from(cause, { phase: 'close', execution: 'unknown' });
        }
    }

    /** `await using` support. */
    [Symbol.asyncDispose](): Promise<void> {
        return this.close();
    }

    private requireReady(operation: string): WireClient {
        if (this.state !== 'ready' || !this.wire) {
            throw new DatabaseError(
                this.state === 'closed' || this.state === 'failed' ? 'yydb.state.closed' : 'yydb.state.invalid',
                `\`${operation}\` requires a connected YydbClient (state: ${this.state})`,
                { phase: 'execute', execution: 'not-sent' },
            );
        }
        return this.wire;
    }
}

function remoteVosGap(method: string): DatabaseError {
    return new DatabaseError(
        'yydb.unsupported',
        `\`${method}\` over the wire needs the Prepare/ExecutePrepared frames (codes 21–24) which the YY wire client and the N-API remote handle do not expose yet`,
        { phase: 'execute', execution: 'not-sent' },
    );
}

async function guardWire<T>(op: () => Promise<T>): Promise<T> {
    try {
        return await op();
    } catch (cause) {
        throw DatabaseError.from(cause, { phase: 'execute', execution: 'replied' });
    }
}

function isCode(value: unknown, code: string): boolean {
    return typeof value === 'object' && value !== null && (value as { code?: unknown }).code === code;
}
