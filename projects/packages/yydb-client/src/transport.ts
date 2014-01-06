import type { Frame } from "./wire.js";

export type Transport = {
    send(frame: Frame): Promise<Frame>;
    close(): void;
};

export function normalizeWsUrl(endpoint: string): string {
    const trimmed = endpoint.trim();
    if (trimmed.startsWith("ws://") || trimmed.startsWith("wss://")) {
        if (trimmed.includes("/wire")) {
            return trimmed;
        }
        return trimmed.replace(/\/$/, "") + "/wire";
    }
    const hostport = trimmed
        .replace(/^tcp:\/\//, "")
        .replace(/^http:\/\//, "")
        .split("/")[0]!;
    return `ws://${hostport}/wire`;
}

export function normalizeTcpHostPort(endpoint: string): string {
    const trimmed = endpoint
        .trim()
        .replace(/^tcp:\/\//, "")
        .replace(/^http:\/\//, "")
        .replace(/^ws:\/\//, "")
        .replace(/^wss:\/\//, "");
    return trimmed.split("/")[0]!;
}
