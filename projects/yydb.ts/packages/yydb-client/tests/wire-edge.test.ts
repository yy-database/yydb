import assert from "node:assert/strict";
import test from "node:test";

import {
    decodeErrorBody,
    decodeFrame,
    decodeKvGetOk,
    decodeMicroHostInvoke,
    decodeScalarCallOk,
    decodeSchemaGetOk,
    encodeErrorBody,
    encodeFrame,
    encodeKvPut,
    encodeMicroHostInvokeOk,
    encodeMicroRegister,
    encodeScalarCall,
    HEADER_LEN,
    MAGIC_YYDS,
    MAX_BODY_LEN,
    MsgType,
    pushBytes,
    pushU32,
    WIRE_VERSION_NEXT,
    type MicroRegisterPayload,
} from "../src/wire.ts";

/**
 * Wire edge coverage: the product-magic and version gates, truncation and
 * oversize guards on both directions, raw-octet fidelity, and the scalar
 * value codec including its malformed-input rejections.
 */

function frameBytes(msgType: number, body: readonly number[], magic: Uint8Array = MAGIC_YYDS, version = "0000"): Uint8Array {
    const out = new Uint8Array(HEADER_LEN + body.length);
    out.set(magic, 0);
    out.set(new TextEncoder().encode(version), 4);
    const view = new DataView(out.buffer);
    view.setUint16(8, msgType, true);
    view.setUint16(10, 7, true);
    view.setUint32(12, 0xdead_beef, true);
    view.setUint32(16, body.length, true);
    out.set(Uint8Array.from(body), HEADER_LEN);
    return out;
}

test("both product magics decode and `YYDS` survives an encode-decode round trip", () => {
    const yyds = encodeFrame({ msgType: MsgType.Info, flags: 0, requestId: 5, body: new Uint8Array() }, "YYDS");
    assert.deepEqual(Array.from(yyds.subarray(0, 4)), Array.from(MAGIC_YYDS));
    assert.deepEqual(Array.from(yyds.subarray(4, 8)), Array.from(new TextEncoder().encode("0000")));

    const decoded = decodeFrame(yyds);
    assert.equal(decoded.product, "YYDS");
    assert.equal(decoded.msgType, MsgType.Info);
    assert.equal(decoded.requestId, 5);

    const yydb = decodeFrame(encodeFrame({ msgType: MsgType.Info, flags: 0, requestId: 5, body: new Uint8Array() }));
    assert.equal(yydb.product, "YYDB");
});

test("the version gate accepts only the spoken wire version", () => {
    assert.throws(
        () => decodeFrame(frameBytes(MsgType.Info, [], MAGIC_YYDS, new TextDecoder().decode(WIRE_VERSION_NEXT))),
        /wire version not supported \(got 0001, want 0000\)/,
    );
    assert.throws(() => decodeFrame(frameBytes(MsgType.Info, [], new TextEncoder().encode("YYXX"))), /bad product magic/);
});

test("truncated headers and lying body lengths are refused", () => {
    const full = encodeFrame({ msgType: MsgType.KvGetOk, flags: 0, requestId: 1, body: Uint8Array.from([9, 8, 7]) });
    assert.throws(() => decodeFrame(full.subarray(0, HEADER_LEN - 1)), /truncated header/);
    assert.throws(() => decodeFrame(full.subarray(0, full.length - 1)), /truncated body/);
});

test("oversized bodies are refused on encode and on decode", () => {
    const huge = new Uint8Array(MAX_BODY_LEN + 1);
    assert.throws(() => encodeFrame({ msgType: MsgType.KvPut, flags: 0, requestId: 1, body: huge }), /wire body too large/);

    const lying = new Uint8Array(HEADER_LEN);
    const view = new DataView(lying.buffer);
    lying.set(MAGIC_YYDS, 0);
    lying.set(new TextEncoder().encode("0000"), 4);
    view.setUint16(8, MsgType.KvPut, true);
    view.setUint32(16, MAX_BODY_LEN + 1, true);
    assert.throws(() => decodeFrame(lying), /wire body too large/);
});

test("headers round-trip full-width ids and flags losslessly", () => {
    const decoded = decodeFrame(frameBytes(MsgType.ScalarCallOk, [3, 5], MAGIC_YYDS));
    assert.equal(decoded.flags, 7);
    assert.equal(decoded.requestId, 0xdead_beef);
});

test("raw octets survive the kv put encode and get-ok decode without text coercion", () => {
    const octets = Uint8Array.from([0x00, 0x80, 0xff, 0x22, 0x5c]);
    const put = encodeKvPut("k", octets);
    // key length prefix (u32 LE = 1), key, value length prefix (u32 LE = 5), value.
    assert.deepEqual(Array.from(put.subarray(0, 5)), [1, 0, 0, 0, 0x6b]);

    const ok = Uint8Array.from([1, ...(() => { const acc: number[] = []; pushBytes(acc, octets); return acc; })()]);
    const decoded = decodeKvGetOk(ok);
    assert.ok(decoded);
    assert.deepEqual(Array.from(decoded), Array.from(octets));
});

test("scalar values round-trip every Phase-1 tag", () => {
    const cases: ReadonlyArray<readonly [unknown, unknown]> = [
        [null, null],
        [true, true],
        [false, false],
        [-42, -42],
        [9007199254740991, 9007199254740991],
        ["", ""],
        ["汉字🎉", "汉字🎉"],
    ];
    for (const [value, expected] of cases) {
        const body = encodeMicroHostInvokeOk(value as never);
        assert.equal(decodeScalarCallOk(body), expected, String(value));
    }
});

test("malformed scalar value bodies are refused with precise messages", () => {
    assert.throws(() => decodeScalarCallOk(new Uint8Array()), /scalar value missing tag/);
    assert.throws(() => decodeScalarCallOk(Uint8Array.from([9])), /invalid type tag/);
    assert.throws(() => decodeScalarCallOk(Uint8Array.from([1])), /bool value truncated/);
    assert.throws(() => decodeScalarCallOk(Uint8Array.from([2, 1, 2, 3])), /i64 value truncated/);
    assert.throws(() => decodeScalarCallOk(Uint8Array.from([3, 8, 0, 0, 0, 1])), /body truncated bytes/);
    assert.throws(() => encodeScalarCall("f", 1, Array.from({ length: 256 }, () => null)), /arg count exceeds u8/);
});

test("schema get-ok decodes absent, present and malformed envelopes", () => {
    assert.throws(() => decodeSchemaGetOk(new Uint8Array()), /schema get ok empty/);
    assert.equal(decodeSchemaGetOk(Uint8Array.from([0])), null);

    const doc = new TextEncoder().encode("table T {\n    @@t_id: i64,\n}");
    const acc: number[] = [1];
    pushU32(acc, 3);
    pushBytes(acc, doc);
    assert.deepEqual(decodeSchemaGetOk(Uint8Array.from(acc)), { version: 3, document: "table T {\n    @@t_id: i64,\n}" });
});

test("kv get-ok distinguishes absent from empty and rejects empty bodies", () => {
    assert.throws(() => decodeKvGetOk(new Uint8Array()), /kv get ok empty/);
    assert.equal(decodeKvGetOk(Uint8Array.from([0])), null);
    const emptyValue = Uint8Array.from([1, 0, 0, 0, 0]);
    assert.deepEqual(decodeKvGetOk(emptyValue), new Uint8Array());
});

test("micro register bodies enforce the fingerprint and arity budgets", () => {
    const base: MicroRegisterPayload = {
        hostId: 4,
        handleVersion: 1,
        udfVersion: 2,
        name: "demo.identity",
        functionId: "7",
        args: ["i64"],
        returns: "i64",
        fingerprint: new Uint8Array(32).fill(0xab),
    };
    assert.throws(() => encodeMicroRegister({ ...base, fingerprint: new Uint8Array(31) }), /fingerprint must be 32 bytes/);
    assert.throws(
        () => encodeMicroRegister({ ...base, args: Array.from({ length: 256 }, () => "i64" as const) }),
        /at most 255 args/,
    );
    assert.throws(() => encodeMicroRegister({ ...base, returns: "f64" as never }), /unsupported wire udf type/);

    const body = encodeMicroRegister(base);
    // hostId u64 + handleVersion u32 + udfVersion u32 + name bytes + functionId bytes + count + tags + fingerprint.
    assert.equal(body.length, 8 + 4 + 4 + (4 + base.name.length) + (4 + base.functionId.length) + 1 + 1 + 1 + 32);
});

test("micro host invoke decodes full payloads and refuses truncated ones", () => {
    const acc: number[] = [];
    const hostView = new DataView(new ArrayBuffer(8));
    hostView.setBigUint64(0, 9n, true);
    acc.push(...new Uint8Array(hostView.buffer));
    pushU32(acc, 2);
    pushBytes(acc, new TextEncoder().encode("fn-1"));
    acc.push(2);
    acc.push(1, 1);
    acc.push(3);
    pushBytes(acc, new TextEncoder().encode("hi"));
    const decoded = decodeMicroHostInvoke(Uint8Array.from(acc));
    assert.deepEqual(decoded, { hostId: 9, handleVersion: 2, functionId: "fn-1", args: [true, "hi"] });

    assert.throws(() => decodeMicroHostInvoke(new Uint8Array(7)), /truncated u64/);
    assert.throws(() => decodeMicroHostInvoke(Uint8Array.from(new Uint8Array(8 + 4 + 4))), /missing count/);
});

test("error bodies carry arbitrary unicode codes and messages", () => {
    const body = encodeErrorBody({ code: "yydb.schema.invalid", message: "VOS schema：字段“名字”重复" });
    assert.deepEqual(decodeErrorBody(body), { code: "yydb.schema.invalid", message: "VOS schema：字段“名字”重复" });
    assert.throws(() => decodeErrorBody(Uint8Array.from([4, 0, 0, 0, 1])), /body truncated bytes/);
});

test("little-endian primitives write u32 and length-prefixed bytes verbatim", () => {
    const acc: number[] = [];
    pushU32(acc, 0x1234_5678);
    assert.deepEqual(acc, [0x78, 0x56, 0x34, 0x12]);
    pushBytes(acc, Uint8Array.from([1, 2, 3]));
    assert.deepEqual(acc, [0x78, 0x56, 0x34, 0x12, 3, 0, 0, 0, 1, 2, 3]);
});
