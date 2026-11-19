import {
    decodeFrame,
    decodeMicroHostInvoke,
    encodeFrame,
    encodeMicroHostInvokeOk,
    HEADER_LEN,
    MAX_BODY_LEN,
    MsgType,
    type Frame,
} from './wire.js';
import { normalizeTcpHostPort, type MicroHostHandler, type Transport } from './transport.js';

async function readExact(readExactBytes: (n: number) => Promise<Uint8Array>, n: number): Promise<Uint8Array> {
    return readExactBytes(n);
}

async function readFrame(readExactBytes: (n: number) => Promise<Uint8Array>): Promise<Frame> {
    const header = await readExact(readExactBytes, HEADER_LEN);
    const bodyLen = new DataView(header.buffer, header.byteOffset, header.byteLength).getUint32(HEADER_LEN - 4, true);
    // 帧上限归上游协议：核心在 encode/decode 两侧都按 `MAX_BODY_LEN` 拒绝，
    // 宿主读帧前先用同一常量拦截，避免为超限帧分配和读取整段 body。
    if (MAX_BODY_LEN > 0 && bodyLen > MAX_BODY_LEN) {
        throw new Error(`frame body length ${bodyLen} exceeds the protocol limit ${MAX_BODY_LEN}`);
    }
    const body = bodyLen > 0 ? await readExact(readExactBytes, bodyLen) : new Uint8Array();
    const full = new Uint8Array(HEADER_LEN + body.byteLength);
    full.set(header, 0);
    full.set(body, HEADER_LEN);
    return decodeFrame(full);
}

/** Node.js TCP transport for `yydb serve` binary frames. */
export async function openTcpTransport(endpoint: string): Promise<Transport> {
    const net = await import('node:net');
    const hostport = normalizeTcpHostPort(endpoint);
    const [host, portText] = hostport.includes(']:')
        ? (() => {
              const rest = hostport.slice(1);
              const idx = rest.indexOf(']:');
              return [rest.slice(0, idx), rest.slice(idx + 2)] as const;
          })()
        : (hostport.split(':') as [string, string]);
    const port = Number(portText);
    if (!host || !Number.isFinite(port)) {
        throw new Error(`invalid TCP endpoint: ${endpoint}`);
    }

    const socket = net.createConnection({ host, port });
    await new Promise<void>((resolve, reject) => {
        socket.once('connect', () => resolve());
        socket.once('error', reject);
    });

    let buffer = new Uint8Array(0);
    const waiters: Array<() => void> = [];
    let microHostHandler: MicroHostHandler | undefined;

    socket.on('data', (chunk: Buffer) => {
        const next = new Uint8Array(buffer.byteLength + chunk.byteLength);
        next.set(buffer, 0);
        next.set(chunk, buffer.byteLength);
        buffer = next;
        for (const wake of waiters.splice(0)) {
            wake();
        }
    });

    async function readExactBytes(n: number): Promise<Uint8Array> {
        while (buffer.byteLength < n) {
            await new Promise<void>((resolve) => waiters.push(resolve));
        }
        const out = buffer.slice(0, n);
        buffer = buffer.slice(n);
        return out;
    }

    async function writeFrame(frame: Frame): Promise<void> {
        const encoded = encodeFrame(frame);
        await new Promise<void>((resolve, reject) => {
            socket.write(Buffer.from(encoded), (error) => {
                if (error) reject(error);
                else resolve();
            });
        });
    }

    async function handleServerPush(frame: Frame): Promise<boolean> {
        if (frame.msgType !== MsgType.MicroHostInvoke) {
            return false;
        }
        if (!microHostHandler) {
            throw new Error('received MicroHostInvoke without a micro host handler');
        }
        const payload = decodeMicroHostInvoke(frame.body);
        const result = await microHostHandler(payload);
        await writeFrame({
            msgType: MsgType.MicroHostInvokeOk,
            flags: 0,
            requestId: frame.requestId,
            body: encodeMicroHostInvokeOk(result),
        });
        return true;
    }

    return {
        setMicroHostHandler(handler) {
            microHostHandler = handler;
        },
        async send(frame) {
            await writeFrame(frame);
            while (true) {
                const response = await readFrame(readExactBytes);
                if (await handleServerPush(response)) {
                    continue;
                }
                return response;
            }
        },
        close() {
            socket.end();
        },
    };
}
