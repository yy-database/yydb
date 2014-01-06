import { decodeFrame, encodeFrame } from "./wire.js";
import { normalizeTcpHostPort, type Transport } from "./transport.js";

/** Node.js TCP transport for `yydb serve` binary frames. */
export async function openTcpTransport(endpoint: string): Promise<Transport> {
    const net = await import("node:net");
    const hostport = normalizeTcpHostPort(endpoint);
    const [host, portText] = hostport.includes("]:")
        ? (() => {
              const rest = hostport.slice(1);
              const idx = rest.indexOf("]:");
              return [rest.slice(0, idx), rest.slice(idx + 2)] as const;
          })()
        : (hostport.split(":") as [string, string]);
    const port = Number(portText);
    if (!host || !Number.isFinite(port)) {
        throw new Error(`invalid TCP endpoint: ${endpoint}`);
    }

    const socket = net.createConnection({ host, port });
    await new Promise<void>((resolve, reject) => {
        socket.once("connect", () => resolve());
        socket.once("error", reject);
    });

    let buffer = new Uint8Array(0);
    const waiters: Array<() => void> = [];

    socket.on("data", (chunk: Buffer) => {
        const next = new Uint8Array(buffer.byteLength + chunk.byteLength);
        next.set(buffer, 0);
        next.set(chunk, buffer.byteLength);
        buffer = next;
        for (const wake of waiters.splice(0)) {
            wake();
        }
    });

    async function readExact(n: number): Promise<Uint8Array> {
        while (buffer.byteLength < n) {
            await new Promise<void>((resolve) => waiters.push(resolve));
        }
        const out = buffer.slice(0, n);
        buffer = buffer.slice(n);
        return out;
    }

    return {
        async send(frame) {
            const encoded = encodeFrame(frame);
            await new Promise<void>((resolve, reject) => {
                socket.write(Buffer.from(encoded), (error) => {
                    if (error) reject(error);
                    else resolve();
                });
            });
            const header = await readExact(20);
            const bodyLen = new DataView(
                header.buffer,
                header.byteOffset,
                header.byteLength,
            ).getUint32(16, true);
            const body = bodyLen > 0 ? await readExact(bodyLen) : new Uint8Array();
            const full = new Uint8Array(20 + body.byteLength);
            full.set(header, 0);
            full.set(body, 20);
            return decodeFrame(full);
        },
        close() {
            socket.end();
        },
    };
}
