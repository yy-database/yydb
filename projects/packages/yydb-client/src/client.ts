import {
    decodeKvGetOk,
    decodeSchemaGetOk,
    encodeKvGet,
    encodeKvPut,
    encodeSchemaEnsure,
    Frame,
    MsgType,
    SchemaVersion,
} from "./wire.js";
import type { Transport } from "./transport.js";
import { openWebSocketTransport } from "./ws.js";

export type {
    Frame,
    MsgTypeCode,
    SchemaVersion,
} from "./wire.js";
export { MsgType, encodeFrame, decodeFrame } from "./wire.js";

export interface ConnectOptions {
    /** `ws://127.0.0.1:7700/wire` or `127.0.0.1:7700` (upgraded to `/wire`). */
    endpoint: string;
}

/**
 * TypeScript client for YY wire (browser = WebSocket `/wire`).
 * Accepts product magic `YYDB` or `YYDS` as equivalent self-claims (same wire);
 * does not require the magic to match a preferred product. Version gate: `0000`.
 */
export class Client {
    readonly endpoint: string;
    private transport: Transport;
    private nextId = 1;

    private constructor(endpoint: string, transport: Transport) {
        this.endpoint = endpoint;
        this.transport = transport;
    }

    /** Construct from an already-open transport (used by Node TCP entry). */
    static fromTransport(endpoint: string, transport: Transport): Client {
        return new Client(endpoint, transport);
    }

    /** Open WebSocket to `yydb serve` `/wire` and Hello-handshake. */
    static async connect(options: ConnectOptions | string): Promise<Client> {
        const endpoint = typeof options === "string" ? options : options.endpoint;
        if (!endpoint.trim()) {
            throw new Error("empty serve endpoint");
        }
        const transport = await openWebSocketTransport(endpoint);
        const client = new Client(endpoint, transport);
        await client.hello();
        return client;
    }

    close() {
        this.transport.close();
    }

    async hello(): Promise<string> {
        const response = await this.roundtrip(MsgType.Hello, new Uint8Array());
        return new TextDecoder().decode(response.body);
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
        const bytes = typeof value === "string" ? new TextEncoder().encode(value) : value;
        await this.roundtrip(MsgType.KvPut, encodeKvPut(key, bytes));
    }

    private async roundtrip(msgType: number, body: Uint8Array): Promise<Frame> {
        const requestId = this.nextId++;
        const response = await this.transport.send({
            msgType,
            flags: 0,
            requestId,
            body,
        });
        if (response.requestId !== requestId) {
            throw new Error("wire response request_id mismatch");
        }
        if (response.msgType === MsgType.Error) {
            throw new Error(new TextDecoder().decode(response.body));
        }
        const expectedOk: Record<number, number> = {
            [MsgType.Hello]: MsgType.HelloOk,
            [MsgType.Info]: MsgType.InfoOk,
            [MsgType.SchemaGet]: MsgType.SchemaGetOk,
            [MsgType.SchemaEnsure]: MsgType.SchemaEnsureOk,
            [MsgType.KvGet]: MsgType.KvGetOk,
            [MsgType.KvPut]: MsgType.KvPutOk,
        };
        const want = expectedOk[msgType];
        if (want !== undefined && response.msgType !== want) {
            throw new Error(`unexpected wire response type ${response.msgType}`);
        }
        return response;
    }
}
