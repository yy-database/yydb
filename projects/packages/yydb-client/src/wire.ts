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
    MicroRegister: 13,
    MicroRegisterOk: 14,
    MicroHostInvoke: 15,
    MicroHostInvokeOk: 16,
    ScalarCall: 17,
    ScalarCallOk: 18,
    Error: 255,
} as const;

/** Phase-1 scalar kinds on the YY wire micro register path. */
export type WireUdfScalarKind = "null" | "bool" | "i64" | "text";

/** Phase-1 scalar value on the YY wire host invoke path. */
export type WireUdfScalarValue =
    | null
    | boolean
    | number
    | string;

export interface MicroHostInvokePayload {
    hostId: number;
    handleVersion: number;
    functionId: string;
    args: readonly WireUdfScalarValue[];
}

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

export interface MicroRegisterPayload {
    hostId: number;
    handleVersion: number;
    udfVersion: number;
    name: string;
    functionId: string;
    args: readonly WireUdfScalarKind[];
    returns: WireUdfScalarKind;
    fingerprint: Uint8Array;
}

function encodeUdfTypeTag(kind: WireUdfScalarKind): number {
    switch (kind) {
        case "null":
            return 0;
        case "bool":
            return 1;
        case "i64":
            return 2;
        case "text":
            return 3;
        default:
            throw new Error(`unsupported wire udf type: ${kind}`);
    }
}

function pushU64(out: number[], value: number) {
    const view = new DataView(new ArrayBuffer(8));
    view.setBigUint64(0, BigInt(value), true);
    out.push(...new Uint8Array(view.buffer));
}

/** Encode a `MicroRegister` body. */
function encodeUdfValue(out: number[], value: WireUdfScalarValue): void {
    if (value === null) {
        out.push(0);
        return;
    }
    if (typeof value === "boolean") {
        out.push(1, value ? 1 : 0);
        return;
    }
    if (typeof value === "number") {
        out.push(2);
        const view = new DataView(new ArrayBuffer(8));
        view.setBigInt64(0, BigInt(value), true);
        out.push(...new Uint8Array(view.buffer));
        return;
    }
    if (typeof value === "string") {
        out.push(3);
        pushBytes(out, new TextEncoder().encode(value));
        return;
    }
    throw new Error("unsupported wire udf value");
}

function decodeUdfValue(body: Uint8Array, offset: { n: number }): WireUdfScalarValue {
    if (offset.n >= body.byteLength) {
        throw new Error("wire scalar value missing tag");
    }
    const tag = body[offset.n]!;
    offset.n += 1;
    switch (tag) {
        case 0:
            return null;
        case 1: {
            if (offset.n >= body.byteLength) {
                throw new Error("wire bool value truncated");
            }
            const value = body[offset.n]! !== 0;
            offset.n += 1;
            return value;
        }
        case 2: {
            if (offset.n + 8 > body.byteLength) {
                throw new Error("wire i64 value truncated");
            }
            const view = new DataView(body.buffer, body.byteOffset, body.byteLength);
            const value = Number(view.getBigInt64(offset.n, true));
            offset.n += 8;
            return value;
        }
        case 3: {
            const len = readU32At(body, offset);
            const bytes = readBytesAt(body, offset, len);
            return new TextDecoder().decode(bytes);
        }
        default:
            throw new Error("wire scalar value has invalid type tag");
    }
}

function encodeUdfValues(values: readonly WireUdfScalarValue[]): number[] {
    if (values.length > 255) {
        throw new Error("wire scalar arg count exceeds u8");
    }
    const out: number[] = [values.length];
    for (const value of values) {
        encodeUdfValue(out, value);
    }
    return out;
}

function decodeUdfValues(body: Uint8Array, offset: { n: number }): WireUdfScalarValue[] {
    if (offset.n >= body.byteLength) {
        throw new Error("wire scalar args missing count");
    }
    const count = body[offset.n]!;
    offset.n += 1;
    const values: WireUdfScalarValue[] = [];
    for (let index = 0; index < count; index += 1) {
        values.push(decodeUdfValue(body, offset));
    }
    return values;
}

/** Decode a `MicroHostInvoke` body. */
export function decodeMicroHostInvoke(body: Uint8Array): MicroHostInvokePayload {
    const offset = { n: 0 };
    const hostId = Number(readU64At(body, offset));
    const handleVersion = readU32At(body, offset);
    const functionIdLen = readU32At(body, offset);
    const functionId = new TextDecoder().decode(readBytesAt(body, offset, functionIdLen));
    const args = decodeUdfValues(body, offset);
    return { hostId, handleVersion, functionId, args };
}

/** Encode a `MicroHostInvokeOk` body. */
export function encodeMicroHostInvokeOk(value: WireUdfScalarValue): Uint8Array {
    const out: number[] = [];
    encodeUdfValue(out, value);
    return Uint8Array.from(out);
}

/** Encode a `ScalarCall` body. */
export function encodeScalarCall(
    name: string,
    version: number,
    args: readonly WireUdfScalarValue[],
): Uint8Array {
    const out: number[] = [];
    pushU32(out, version >>> 0);
    pushBytes(out, new TextEncoder().encode(name));
    out.push(...encodeUdfValues(args));
    return Uint8Array.from(out);
}

/** Decode a `ScalarCallOk` body. */
export function decodeScalarCallOk(body: Uint8Array): WireUdfScalarValue {
    return decodeUdfValue(body, { n: 0 });
}

function readU64At(body: Uint8Array, offset: { n: number }): bigint {
    if (offset.n + 8 > body.byteLength) {
        throw new Error("wire body truncated u64");
    }
    const view = new DataView(body.buffer, body.byteOffset, body.byteLength);
    const value = view.getBigUint64(offset.n, true);
    offset.n += 8;
    return value;
}

export function encodeMicroRegister(payload: MicroRegisterPayload): Uint8Array {
    if (payload.fingerprint.byteLength !== 32) {
        throw new Error("micro register fingerprint must be 32 bytes");
    }
    const out: number[] = [];
    pushU64(out, payload.hostId);
    pushU32(out, payload.handleVersion);
    pushU32(out, payload.udfVersion);
    pushBytes(out, new TextEncoder().encode(payload.name));
    pushBytes(out, new TextEncoder().encode(payload.functionId));
    if (payload.args.length > 255) {
        throw new Error("micro register supports at most 255 args");
    }
    out.push(payload.args.length);
    for (const arg of payload.args) {
        out.push(encodeUdfTypeTag(arg));
    }
    out.push(encodeUdfTypeTag(payload.returns));
    out.push(...payload.fingerprint);
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
