import { Client, type ConnectOptions } from "./client.js";
import { openTcpTransport } from "./tcp.js";
import { openWebSocketTransport } from "./ws.js";
import { normalizeWsUrl } from "./transport.js";

export { Client } from "./client.js";
export type { ConnectOptions } from "./client.js";
export {
    MsgType,
    encodeFrame,
    decodeFrame,
    encodeSchemaEnsure,
    encodeKvGet,
    encodeKvPut,
    decodeSchemaGetOk,
    decodeKvGetOk,
} from "./wire.js";
export type { Frame, MsgTypeCode, SchemaVersion } from "./wire.js";

function wantsWebSocket(endpoint: string): boolean {
    const trimmed = endpoint.trim();
    return trimmed.startsWith("ws://") || trimmed.startsWith("wss://");
}

/** Node entry: TCP by default; `ws://` uses WebSocket. */
export async function connect(options: ConnectOptions | string): Promise<Client> {
    const endpoint = typeof options === "string" ? options : options.endpoint;
    if (!endpoint.trim()) {
        throw new Error("empty serve endpoint");
    }
    const transport = wantsWebSocket(endpoint)
        ? await openWebSocketTransport(endpoint)
        : await openTcpTransport(endpoint);
    const client = Client.fromTransport(
        wantsWebSocket(endpoint) ? normalizeWsUrl(endpoint) : endpoint,
        transport,
    );
    await client.hello();
    return client;
}
