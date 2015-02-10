import { decodeFrame, decodeMicroHostInvoke, encodeFrame, encodeMicroHostInvokeOk, MsgType, type Frame } from './wire.js';
import { normalizeWsUrl, type MicroHostHandler, type Transport } from './transport.js';

/** Browser / any host with WebSocket — path `/wire`, binary frames. */
export async function openWebSocketTransport(endpoint: string): Promise<Transport> {
    const url = normalizeWsUrl(endpoint);
    const ws = new WebSocket(url);
    ws.binaryType = 'arraybuffer';

    await new Promise<void>((resolve, reject) => {
        ws.addEventListener('open', () => resolve(), { once: true });
        ws.addEventListener('error', () => reject(new Error(`websocket failed: ${url}`)), {
            once: true,
        });
    });

    let microHostHandler: MicroHostHandler | undefined;
    const buffer: Frame[] = [];
    const waiters: Array<() => void> = [];

    ws.addEventListener('message', (event) => {
        const data = event.data instanceof ArrayBuffer ? new Uint8Array(event.data) : new Uint8Array(event.data as ArrayBuffer);
        buffer.push(decodeFrame(data));
        for (const wake of waiters.splice(0)) {
            wake();
        }
    });

    async function readFrame(): Promise<Frame> {
        while (buffer.length === 0) {
            await new Promise<void>((resolve) => waiters.push(resolve));
        }
        return buffer.shift()!;
    }

    async function writeFrame(frame: Frame): Promise<void> {
        ws.send(encodeFrame(frame));
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
                const response = await readFrame();
                if (await handleServerPush(response)) {
                    continue;
                }
                return response;
            }
        },
        close() {
            ws.close();
        },
    };
}
