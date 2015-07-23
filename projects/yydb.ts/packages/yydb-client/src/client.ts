import {
    decodeAuthenticateOk,
    decodeErrorBody,
    decodeKvGetOk,
    decodeScalarCallOk,
    decodeSchemaGetOk,
    encodeAuthenticate,
    encodeKvGet,
    encodeKvPut,
    encodeMicroRegister,
    encodeScalarCall,
    encodeSchemaEnsure,
    Frame,
    MicroRegisterPayload,
    MsgType,
    SchemaVersion,
    WireUdfScalarValue,
    type WireErrorBody,
} from './wire.js';
import type { MicroHostHandler, Transport } from './transport.js';
import { openWebSocketTransport } from './ws.js';
import { YydbError, YydbErrorCodes } from './errors.js';

export type {
    Frame,
    MicroHostInvokePayload,
    MicroRegisterPayload,
    MsgTypeCode,
    SchemaVersion,
    WireErrorBody,
    WireUdfScalarKind,
    WireUdfScalarValue,
} from './wire.js';
export type { MicroHostHandler } from './transport.js';
export {
    MsgType,
    encodeFrame,
    decodeFrame,
    encodeAuthenticate,
    decodeAuthenticateOk,
    encodeErrorBody,
    decodeErrorBody,
} from './wire.js';
export { YydbError, YydbErrorCodes, YydbClientErrorCodes, isYydbError } from './errors.js';
export type { YydbErrorCode, YydbErrorOptions } from './errors.js';

export interface ConnectOptions {
    /** `ws://127.0.0.1:7700/wire` or `127.0.0.1:7700` (upgraded to `/wire`). */
    endpoint: string;
    /**
     * Bearer token sent in the `Authenticate` frame right after Hello.
     * Omit to stay unauthenticated — `yydb serve` rejects every other frame
     * with `yydb.auth.token_invalid` until an `Authenticate` succeeds.
     */
    token?: string;
}

/** Expected reply type and operation label per request type. */
const REQUESTS: Record<number, { operation: string; ok: number }> = {
    [MsgType.Hello]: { operation: 'hello', ok: MsgType.HelloOk },
    [MsgType.Authenticate]: { operation: 'authenticate', ok: MsgType.AuthenticateOk },
    [MsgType.Info]: { operation: 'info', ok: MsgType.InfoOk },
    [MsgType.SchemaGet]: { operation: 'schemaGet', ok: MsgType.SchemaGetOk },
    [MsgType.SchemaEnsure]: { operation: 'schemaEnsure', ok: MsgType.SchemaEnsureOk },
    [MsgType.KvGet]: { operation: 'kvGet', ok: MsgType.KvGetOk },
    [MsgType.KvPut]: { operation: 'kvPut', ok: MsgType.KvPutOk },
    [MsgType.MicroRegister]: { operation: 'microRegister', ok: MsgType.MicroRegisterOk },
    [MsgType.ScalarCall]: { operation: 'scalarCall', ok: MsgType.ScalarCallOk },
};

/**
 * TypeScript client for YY wire (browser = WebSocket `/wire`).
 * Accepts product magic `YYDB` or `YYDS` as equivalent self-claims (same wire);
 * does not require the magic to match a preferred product. Version gate: `0000`.
 */
export class Client {
    readonly endpoint: string;
    private transport: Transport;
    private nextId = 1;
    private closed = false;

    private constructor(endpoint: string, transport: Transport) {
        this.endpoint = endpoint;
        this.transport = transport;
    }

    /** Construct from an already-open transport (used by Node TCP entry). */
    static fromTransport(endpoint: string, transport: Transport): Client {
        return new Client(endpoint, transport);
    }

    /** Open WebSocket to `yydb serve` `/wire`, Hello-handshake and optional Authenticate. */
    static async connect(options: ConnectOptions | string): Promise<Client> {
        const endpoint = typeof options === 'string' ? options : options.endpoint;
        const token = typeof options === 'string' ? undefined : options.token;
        if (!endpoint.trim()) {
            throw new YydbError(YydbErrorCodes.clientInvalidEndpoint, 'empty serve endpoint', {
                operation: 'connect',
            });
        }
        let transport: Transport;
        try {
            transport = await openWebSocketTransport(endpoint);
        } catch (cause) {
            throw new YydbError(YydbErrorCodes.clientTransportFailure, `connect ${endpoint}: transport failed`, {
                operation: 'connect',
                retryable: true,
                cause,
            });
        }
        const client = new Client(endpoint, transport);
        await client.hello();
        if (token !== undefined) {
            await client.authenticate(token);
        }
        return client;
    }

    /** Close the transport. Idempotent — further calls fail with `yydb.session.closed`. */
    close(): void {
        if (this.closed) {
            return;
        }
        this.closed = true;
        this.transport.close();
    }

    /** Whether {@link close} already ran. */
    isClosed(): boolean {
        return this.closed;
    }

    async hello(): Promise<string> {
        const response = await this.roundtrip(MsgType.Hello, new Uint8Array());
        return new TextDecoder().decode(response.body);
    }

    /** Send the `Authenticate` frame; resolves with the session token. */
    async authenticate(token: string): Promise<string> {
        const response = await this.roundtrip(MsgType.Authenticate, encodeAuthenticate(token));
        return decodeAuthenticateOk(response.body);
    }

    async serverVersion(): Promise<string> {
        return this.hello();
    }

    async info(): Promise<string> {
        const response = await this.roundtrip(MsgType.Info, new Uint8Array());
        return new TextDecoder().decode(response.body);
    }

    async getSchema(): Promise<SchemaVersion | null> {
        const response = await this.roundtrip(MsgType.SchemaGet, new Uint8Array());
        return decodeSchemaGetOk(response.body);
    }

    async ensureSchema(version: number, document: string): Promise<void> {
        await this.roundtrip(MsgType.SchemaEnsure, encodeSchemaEnsure(version, document));
    }

    async get(key: string): Promise<Uint8Array | null> {
        const response = await this.roundtrip(MsgType.KvGet, encodeKvGet(key));
        return decodeKvGetOk(response.body);
    }

    async put(key: string, value: Uint8Array | string): Promise<void> {
        const bytes = typeof value === 'string' ? new TextEncoder().encode(value) : value;
        await this.roundtrip(MsgType.KvPut, encodeKvPut(key, bytes));
    }

    /** Register a session-local host micro contract on the remote engine. */
    async registerMicro(payload: MicroRegisterPayload): Promise<void> {
        await this.roundtrip(MsgType.MicroRegister, encodeMicroRegister(payload));
    }

    /** Install the host handler used to answer `MicroHostInvoke` server pushes. */
    setMicroHostHandler(handler: MicroHostHandler | undefined): void {
        this.transport.setMicroHostHandler?.(handler);
    }

    /** Invoke a registered scalar UDF on the remote engine. */
    async callScalar(name: string, version: number, args: readonly WireUdfScalarValue[]): Promise<WireUdfScalarValue> {
        const response = await this.roundtrip(MsgType.ScalarCall, encodeScalarCall(name, version, args));
        return decodeScalarCallOk(response.body);
    }

    private async roundtrip(msgType: number, body: Uint8Array): Promise<Frame> {
        const request = REQUESTS[msgType];
        const operation = request?.operation ?? `msgType(${msgType})`;
        if (this.closed) {
            throw new YydbError(YydbErrorCodes.sessionClosed, `${operation}: YYDB session is closed`, { operation });
        }
        const requestId = this.nextId++;
        let response: Frame;
        try {
            response = await this.transport.send({
                msgType,
                flags: 0,
                requestId,
                body,
            });
        } catch (cause) {
            throw new YydbError(YydbErrorCodes.clientTransportFailure, `${operation}: transport failed`, { operation, retryable: true, cause });
        }
        if (response.requestId !== requestId) {
            throw new YydbError(
                YydbErrorCodes.clientRequestIdMismatch,
                `${operation}: wire response request_id mismatch (got ${response.requestId}, want ${requestId})`,
                { operation },
            );
        }
        if (response.msgType === MsgType.Error) {
            throw remoteError(response.body, operation);
        }
        if (request && response.msgType !== request.ok) {
            throw new YydbError(YydbErrorCodes.clientUnexpectedResponse, `${operation}: unexpected wire response type ${response.msgType}`, {
                operation,
            });
        }
        return response;
    }
}

/** Turn an `Error` frame into a {@link YydbError}, keeping the engine code. */
function remoteError(body: Uint8Array, operation: string): YydbError {
    let decoded: WireErrorBody;
    try {
        decoded = decodeErrorBody(body);
    } catch (cause) {
        return new YydbError(YydbErrorCodes.clientDecodeFailure, `${operation}: malformed wire error frame`, { operation, cause });
    }
    return new YydbError(decoded.code, decoded.message, { operation });
}
