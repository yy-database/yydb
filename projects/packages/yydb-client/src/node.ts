import { Client, type ConnectOptions } from './client.js';
import { openTcpTransport } from './tcp.js';
import { openWebSocketTransport } from './ws.js';
import { normalizeWsUrl } from './transport.js';
import { YydbError, YydbErrorCodes } from './errors.js';

export { Client } from './client.js';
export type { ConnectOptions } from './client.js';
export {
    MsgType,
    encodeFrame,
    decodeFrame,
    encodeSchemaEnsure,
    encodeKvGet,
    encodeKvPut,
    decodeSchemaGetOk,
    decodeKvGetOk,
    encodeErrorBody,
    decodeErrorBody,
} from './wire.js';
export type { Frame, MsgTypeCode, SchemaVersion, WireErrorBody } from './wire.js';
export { YydbError, YydbErrorCodes, isRetryableYydbCode, isYydbError } from './errors.js';
export type { YydbErrorCode, YydbErrorOptions } from './errors.js';
export type { MicroHostHandler, Transport } from './transport.js';

function wantsWebSocket(endpoint: string): boolean {
    const trimmed = endpoint.trim();
    return trimmed.startsWith('ws://') || trimmed.startsWith('wss://');
}

/** Node entry: TCP by default; `ws://` uses WebSocket. */
export async function connect(options: ConnectOptions | string): Promise<Client> {
    const endpoint = typeof options === 'string' ? options : options.endpoint;
    if (!endpoint.trim()) {
        throw new YydbError(YydbErrorCodes.clientInvalidEndpoint, 'empty serve endpoint', {
            operation: 'connect',
        });
    }
    let transport;
    try {
        transport = wantsWebSocket(endpoint) ? await openWebSocketTransport(endpoint) : await openTcpTransport(endpoint);
    } catch (cause) {
        throw new YydbError(YydbErrorCodes.clientTransportFailure, `connect ${endpoint}: transport failed`, { operation: 'connect', cause });
    }
    const client = Client.fromTransport(wantsWebSocket(endpoint) ? normalizeWsUrl(endpoint) : endpoint, transport);
    await client.hello();
    return client;
}
