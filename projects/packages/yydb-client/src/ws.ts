import { decodeFrame, encodeFrame, type Frame } from "./wire.js";
import { normalizeWsUrl, type Transport } from "./transport.js";

/** Browser / any host with WebSocket — path `/wire`, binary frames. */
export async function openWebSocketTransport(endpoint: string): Promise<Transport> {
    const url = normalizeWsUrl(endpoint);
    const ws = new WebSocket(url);
    ws.binaryType = "arraybuffer";

    await new Promise<void>((resolve, reject) => {
        ws.addEventListener("open", () => resolve(), { once: true });
        ws.addEventListener("error", () => reject(new Error(`websocket failed: ${url}`)), {
            once: true,
        });
    });

    let pending: { resolve: (frame: Frame) => void; reject: (error: Error) => void } | null = null;

    ws.addEventListener("message", (event) => {
        try {
            const data =
                event.data instanceof ArrayBuffer
                    ? new Uint8Array(event.data)
                    : new Uint8Array(event.data as ArrayBuffer);
            const frame = decodeFrame(data);
            if (pending) {
                const slot = pending;
                pending = null;
                slot.resolve(frame);
            }
        } catch (error) {
            if (pending) {
                const slot = pending;
                pending = null;
                slot.reject(error as Error);
            }
        }
    });

    return {
        send(frame) {
            return new Promise((resolve, reject) => {
                if (pending) {
                    reject(new Error("overlapping websocket requests"));
                    return;
                }
                pending = { resolve, reject };
                ws.send(encodeFrame(frame));
            });
        },
        close() {
            ws.close();
        },
    };
}
