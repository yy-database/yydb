/**
 * YY wire protocol — VOS-native serve frames.
 * Prefix: product magic `YYDB` | `YYDS` (backend self-claim only; same effect),
 * then four ASCII version digits (`0000` current, `0001` next).
 * Frontends accept either magic and do not require product consistency.
 */

export const MAGIC_YYDB = new TextEncoder().encode("YYDB");
export const MAGIC_YYDS = new TextEncoder().encode("YYDS");
/** @deprecated Use MAGIC_YYDB — encode default product. */
export const MAGIC = MAGIC_YYDB;

/** Current wire version digits. */
export const WIRE_VERSION = new TextEncoder().encode("0000");
/** Next planned wire version (not spoken yet). */
export const WIRE_VERSION_NEXT = new TextEncoder().encode("0001");

export const HEADER_LEN = 20;
export const MAX_BODY_LEN = 16 * 1024 * 1024;

export const MsgType = {
    Hello: 1,
    HelloOk: 2,
    Info: 3,
    InfoOk: 4,
    SchemaGet: 5,
    SchemaGetOk: 6,
    SchemaEnsure: 7,
    SchemaEnsureOk: 8,
    KvGet: 9,
    KvGetOk: 10,
    KvPut: 11,
    KvPutOk: 12,
    Error: 255,
} as const;

export type MsgTypeCode = (typeof MsgType)[keyof typeof MsgType];

export type ProductMagic = "YYDB" | "YYDS";

export interface Frame {
    msgType: number;
    flags: number;
    requestId: number;
    body: Uint8Array;
    /** Product magic observed on decode (backend self-claim; optional to ignore). */
    product?: ProductMagic;
}

function writeU16(view: DataView, offset: number, value: number) {
    view.setUint16(offset, value, true);
}

function writeU32(view: DataView, offset: number, value: number) {
    view.setUint32(offset, value, true);
}

function readU16(view: DataView, offset: number) {
    return view.getUint16(offset, true);
}

function readU32(view: DataView, offset: number) {
    return view.getUint32(offset, true);
}

function productBytes(product: ProductMagic): Uint8Array {
    return product === "YYDS" ? MAGIC_YYDS : MAGIC_YYDB;
}

function parseProduct(bytes: Uint8Array): ProductMagic {
    const text = new TextDecoder().decode(bytes.subarray(0, 4));
    if (text === "YYDB" || text === "YYDS") {
        return text;
    }
    throw new Error("wire frame bad product magic (want YYDB|YYDS)");
}

function checkVersion(bytes: Uint8Array) {
    const version = new TextDecoder().decode(bytes.subarray(4, 8));
    if (version !== "0000") {
        throw new Error(`wire version not supported (got ${version}, want 0000)`);
    }
}

export function encodeFrame(frame: Frame, product: ProductMagic = "YYDB"): Uint8Array {
    if (frame.body.byteLength > MAX_BODY_LEN) {
        throw new Error("wire body too large");
    }
    const out = new Uint8Array(HEADER_LEN + frame.body.byteLength);
    out.set(productBytes(product), 0);
    out.set(WIRE_VERSION, 4);
    const view = new DataView(out.buffer);
    writeU16(view, 8, frame.msgType);
    writeU16(view, 10, frame.flags);
    writeU32(view, 12, frame.requestId);
    writeU32(view, 16, frame.body.byteLength);
    out.set(frame.body, HEADER_LEN);
    return out;
}

export function decodeFrame(bytes: Uint8Array): Frame {
    if (bytes.byteLength < HEADER_LEN) {
        throw new Error("wire frame truncated header");
    }
    const product = parseProduct(bytes);
    checkVersion(bytes);
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const msgType = readU16(view, 8);
    const flags = readU16(view, 10);
    const requestId = readU32(view, 12);
    const bodyLen = readU32(view, 16);
    if (bodyLen > MAX_BODY_LEN) {
        throw new Error("wire body too large");
    }
    const need = HEADER_LEN + bodyLen;
    if (bytes.byteLength < need) {
        throw new Error("wire frame truncated body");
    }
    return {
        msgType,
        flags,
        requestId,
        body: bytes.slice(HEADER_LEN, need),
        product,
    };
}

export function pushU32(out: number[], value: number) {
    out.push(value & 0xff, (value >> 8) & 0xff, (value >> 16) & 0xff, (value >> 24) & 0xff);
}

export function pushBytes(out: number[], bytes: Uint8Array) {
    pushU32(out, bytes.byteLength);
    for (let i = 0; i < bytes.byteLength; i++) {
        out.push(bytes[i]!);
    }
}

export function encodeSchemaEnsure(version: number, document: string): Uint8Array {
    const out: number[] = [];
    pushU32(out, version >>> 0);
    pushBytes(out, new TextEncoder().encode(document));
    return Uint8Array.from(out);
}

export function encodeKvGet(key: string): Uint8Array {
    const out: number[] = [];
    pushBytes(out, new TextEncoder().encode(key));
    return Uint8Array.from(out);
}

export function encodeKvPut(key: string, value: Uint8Array): Uint8Array {
    const out: number[] = [];
    pushBytes(out, new TextEncoder().encode(key));
    pushBytes(out, value);
    return Uint8Array.from(out);
}

function readU32At(body: Uint8Array, offset: { n: number }): number {
    if (offset.n + 4 > body.byteLength) {
        throw new Error("wire body truncated u32");
    }
    const view = new DataView(body.buffer, body.byteOffset, body.byteLength);
    const value = readU32(view, offset.n);
    offset.n += 4;
    return value;
}

function readBytesAt(body: Uint8Array, offset: { n: number }, len: number): Uint8Array {
    if (offset.n + len > body.byteLength) {
        throw new Error("wire body truncated bytes");
    }
    const slice = body.slice(offset.n, offset.n + len);
    offset.n += len;
    return slice;
}

export interface SchemaVersion {
    version: number;
    document: string;
}

export function decodeSchemaGetOk(body: Uint8Array): SchemaVersion | null {
    if (body.byteLength === 0) {
        throw new Error("schema get ok empty");
    }
    if (body[0] === 0) {
        return null;
    }
    const offset = { n: 1 };
    const version = readU32At(body, offset);
    const docLen = readU32At(body, offset);
    const doc = readBytesAt(body, offset, docLen);
    return { version, document: new TextDecoder().decode(doc) };
}

export function decodeKvGetOk(body: Uint8Array): Uint8Array | null {
    if (body.byteLength === 0) {
        throw new Error("kv get ok empty");
    }
    if (body[0] === 0) {
        return null;
    }
    const offset = { n: 1 };
    const valLen = readU32At(body, offset);
    return readBytesAt(body, offset, valLen);
}
