import { MsgType, decodeFrame, encodeFrame } from "@yydb/yydb-client";

export function toHex(bytes: Uint8Array): string {
    return [...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join(" ");
}

export function encodeHello(requestId: number, bodyText: string): { hex: string; bytes: Uint8Array } {
    const body = new TextEncoder().encode(bodyText);
    const bytes = encodeFrame({
        msgType: MsgType.Hello,
        flags: 0,
        requestId: requestId >>> 0,
        body,
    });
    return { hex: toHex(bytes), bytes };
}

export function decodeHelloHex(hex: string): string {
    const bytes = Uint8Array.from(
        hex.split(/\s+/).filter(Boolean).map((part) => Number.parseInt(part, 16)),
    );
    const frame = decodeFrame(bytes);
    return JSON.stringify(
        {
            msgType: frame.msgType,
            flags: frame.flags,
            requestId: frame.requestId,
            body: new TextDecoder().decode(frame.body),
        },
        null,
        2,
    );
}
