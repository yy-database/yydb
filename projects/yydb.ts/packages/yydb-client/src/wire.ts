/**
 * YY wire protocol — VOS-native serve frames.
 *
 * The wire is owned by the Rust core (`database.protocol.yy`, shipped inside
 * `@yydb/yydb-unknown-wasm32`): frame layout, header bounds, version digits,
 * product magics and every body codec live upstream. This module only maps
 * those exports onto the host-side shapes and keeps the message-type labels.
 *
 * Call `initWire()` (or the platform package's `initWasm()`) before using any
 * codec here. The constants below are filled from the core during `initWire()`.
 */

import { initWasm, wire as core, type InitWasmOptions } from '@yydb/yydb-unknown-wasm32';

/** Product magic the backend self-claims (same protocol either way). */
export type ProductMagic = 'YYDB' | 'YYDS';

/** Four ASCII bytes of the current wire version (`"0000"`). */
export let WIRE_VERSION = new Uint8Array(4);
/** Four ASCII bytes of the next planned wire version (`"0001"`, not spoken yet). */
export let WIRE_VERSION_NEXT = new Uint8Array(4);
/** Product magic bytes for `"YYDB"`. */
export let MAGIC_YYDB = new Uint8Array(4);
/** Product magic bytes for `"YYDS"`. */
export let MAGIC_YYDS = new Uint8Array(4);
/** Absolute frame header length in bytes. */
export let HEADER_LEN = 0;
/** Body soft limit enforced by the core on encode and decode. */
export let MAX_BODY_LEN = 0;

/**
 * Initialize the wasm core backing every codec in this module.
 *
 * Browser bundles load the packaged `lib/yydb_wasm_bg.wasm` automatically;
 * Node hosts pass the wasm bytes (or a URL) through `options`.
 */
export async function initWire(options: InitWasmOptions = {}): Promise<void> {
    await initWasm(options);
    const encoder = new TextEncoder();
    WIRE_VERSION = encoder.encode(core.version());
    WIRE_VERSION_NEXT = encoder.encode(core.versionNext());
    MAGIC_YYDB = core.magic('YYDB');
    MAGIC_YYDS = core.magic('YYDS');
    HEADER_LEN = core.headerLen();
    MAX_BODY_LEN = core.maxBodyLen();
}

/** Message type codes as defined by the upstream `MessageType` table. */
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
    Authenticate: 19,
    AuthenticateOk: 20,
    Error: 255,
} as const;

export type MsgTypeCode = (typeof MsgType)[keyof typeof MsgType];

/**
 * Response type a request code must be answered with, or `null` when the
 * code is not a request. Owned by the core.
 */
export function responseType(code: number): number | null {
    return core.responseType(code);
}

export interface Frame {
    msgType: number;
    flags: number;
    requestId: number;
    body: Uint8Array;
    /** Product magic observed on decode (backend self-claim; optional to ignore). */
    product?: ProductMagic;
}

/** Encode one frame; the core validates the magic, version and body limit. */
export function encodeFrame(frame: Frame, product: ProductMagic = 'YYDB'): Uint8Array {
    return core.frameEncode(frame.msgType, frame.flags, frame.requestId, frame.body, product);
}

/** Decode one complete frame; partial or corrupt buffers throw the core error. */
export function decodeFrame(bytes: Uint8Array): Frame {
    const decoded = core.frameDecode(bytes);
    return {
        msgType: decoded.msgType,
        flags: decoded.flags,
        requestId: decoded.requestId,
        body: decoded.body,
        product: decoded.product as ProductMagic,
    };
}

/** Phase-1 scalar kinds on the YY wire micro register path. */
export type WireUdfScalarKind = 'null' | 'bool' | 'i64' | 'text';

/** Phase-1 scalar value on the YY wire host invoke path. */
export type WireUdfScalarValue = null | boolean | number | string;

export interface SchemaVersion {
    version: number;
    document: string;
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

export interface MicroHostInvokePayload {
    hostId: number;
    handleVersion: number;
    functionId: string;
    args: readonly WireUdfScalarValue[];
}

/** Encode a `SchemaEnsure` body. */
export function encodeSchemaEnsure(version: number, document: string): Uint8Array {
    return core.encodeSchemaEnsure(version, document);
}

/** Decode a `SchemaEnsure` body (round-trip and server-side use). */
export function decodeSchemaEnsure(body: Uint8Array): SchemaVersion {
    return JSON.parse(core.decodeSchemaEnsure(body)) as SchemaVersion;
}

/** Encode a `KvGet` body. */
export function encodeKvGet(key: string): Uint8Array {
    return core.encodeKvGet(key);
}

/** Decode a `KvGet` body (the key). */
export function decodeKvGet(body: Uint8Array): string {
    return core.decodeKvGet(body);
}

/** Encode a `KvPut` body. */
export function encodeKvPut(key: string, value: Uint8Array): Uint8Array {
    return core.encodeKvPut(key, value);
}

/** Decode a `KvPut` body (round-trip and server-side use). */
export function decodeKvPut(body: Uint8Array): { key: string; value: Uint8Array } {
    const parsed = JSON.parse(core.decodeKvPut(body)) as { key: string; value: number[] };
    return { key: parsed.key, value: Uint8Array.from(parsed.value) };
}

/** Encode an `Authenticate` request body: raw bearer token (opaque). */
export function encodeAuthenticate(token: string): Uint8Array {
    return core.encodeAuthenticate(token);
}

/** Decode an `AuthenticateOk` reply body: the session token. */
export function decodeAuthenticateOk(body: Uint8Array): string {
    return core.decodeAuthenticateOk(body);
}

/** Decode a `SchemaGetOk` reply body; `null` means no schema yet. */
export function decodeSchemaGetOk(body: Uint8Array): SchemaVersion | null {
    const decoded = core.decodeSchemaGetOk(body);
    return decoded == null ? null : (JSON.parse(decoded) as SchemaVersion);
}

/** Decode a `KvGetOk` reply body; `null` means the key is absent. */
export function decodeKvGetOk(body: Uint8Array): Uint8Array | null {
    return core.decodeKvGetOk(body) ?? null;
}

/** Encode a `ScalarCall` body. */
export function encodeScalarCall(
    name: string,
    version: number,
    args: readonly WireUdfScalarValue[],
): Uint8Array {
    return core.encodeScalarCall(JSON.stringify({ name, version, args }));
}

/** Decode a `ScalarCallOk` body. */
export function decodeScalarCallOk(body: Uint8Array): WireUdfScalarValue {
    return JSON.parse(core.decodeScalarCallOk(body)) as WireUdfScalarValue;
}

/** Encode a `MicroRegister` body. */
export function encodeMicroRegister(payload: MicroRegisterPayload): Uint8Array {
    return core.encodeMicroRegister(
        JSON.stringify({
            name: payload.name,
            version: payload.udfVersion,
            argTypes: payload.args,
            returnType: payload.returns,
            hostId: payload.hostId,
            functionId: payload.functionId,
            handleVersion: payload.handleVersion,
            fingerprint: Array.from(payload.fingerprint),
        }),
    );
}

/** Decode a `MicroRegister` body (round-trip and server-side use). */
export function decodeMicroRegister(body: Uint8Array): MicroRegisterPayload {
    const decoded = JSON.parse(core.decodeMicroRegister(body)) as {
        name: string;
        version: number;
        argTypes: WireUdfScalarKind[];
        returnType: WireUdfScalarKind;
        hostId: number;
        functionId: string;
        handleVersion: number;
        fingerprint: number[];
    };
    return {
        hostId: decoded.hostId,
        handleVersion: decoded.handleVersion,
        udfVersion: decoded.version,
        name: decoded.name,
        functionId: decoded.functionId,
        args: decoded.argTypes,
        returns: decoded.returnType,
        fingerprint: Uint8Array.from(decoded.fingerprint),
    };
}

/** Decode a `MicroHostInvoke` body. */
export function decodeMicroHostInvoke(body: Uint8Array): MicroHostInvokePayload {
    const decoded = JSON.parse(core.decodeMicroHostInvoke(body)) as {
        hostId: number;
        handleVersion: number;
        functionId: string;
        args: WireUdfScalarValue[];
    };
    return {
        hostId: decoded.hostId,
        handleVersion: decoded.handleVersion,
        functionId: decoded.functionId,
        args: decoded.args,
    };
}

/** Encode a `MicroHostInvokeOk` body. */
export function encodeMicroHostInvokeOk(value: WireUdfScalarValue): Uint8Array {
    return core.encodeMicroHostInvokeOk(JSON.stringify(value));
}

/** Structured `Error` frame body — mirrors the Rust `DiagnosticEnvelope`. */
export interface WireErrorBody {
    /** Stable dotted code (`yydb.schema.invalid`, …). */
    code: string;
    /** Operation category (`io`, `schema`, `udf`, …). Required by the core. */
    operation: string;
    /** Whether the caller may retry with the same arguments. Required by the core. */
    retryable: boolean;
    /** Display message; hosts must not branch on this text. */
    message: string;
    /** Structured parameters for catalog templating. */
    params?: Readonly<Record<string, string>>;
}

/** Encode an `Error` frame body as the shared diagnostic JSON (validated by the core). */
export function encodeErrorBody(error: WireErrorBody): Uint8Array {
    return core.encodeErrorBody(JSON.stringify(error));
}

/** Decode an `Error` frame body; the core enforces the diagnostic envelope contract. */
export function decodeErrorBody(body: Uint8Array): WireErrorBody {
    return JSON.parse(core.decodeErrorBody(body)) as WireErrorBody;
}
