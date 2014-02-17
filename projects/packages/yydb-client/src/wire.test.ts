import assert from "node:assert/strict";
import test from "node:test";
import {
    decodeFrame,
    encodeFrame,
    encodeKvPut,
    decodeKvGetOk,
    encodeMicroRegister,
    MsgType,
} from "./wire.ts";

test("frame roundtrip", () => {
    const encoded = encodeFrame({
        msgType: MsgType.Hello,
        flags: 0,
        requestId: 9,
        body: new TextEncoder().encode("x"),
    });
    assert.equal(new TextDecoder().decode(encoded.subarray(0, 4)), "YYDB");
    assert.equal(new TextDecoder().decode(encoded.subarray(4, 8)), "0000");
    const decoded = decodeFrame(encoded);
    assert.equal(decoded.msgType, MsgType.Hello);
    assert.equal(decoded.requestId, 9);
    assert.equal(decoded.product, "YYDB");
    assert.equal(new TextDecoder().decode(decoded.body), "x");
});

test("kv put body encodes lengths", () => {
    const body = encodeKvPut("a", new TextEncoder().encode("bc"));
    assert.ok(body.byteLength >= 4 + 1 + 4 + 2);
});

test("micro register body encodes contract fields", () => {
    const body = encodeMicroRegister({
        hostId: 9,
        handleVersion: 1,
        udfVersion: 2,
        name: "text.normalize",
        functionId: "3",
        args: ["text"],
        returns: "text",
        fingerprint: Uint8Array.from({ length: 32 }, (_, index) => index),
    });
    assert.equal(body.byteLength, 8 + 4 + 4 + (4 + 14) + (4 + 1) + 1 + 1 + 1 + 32);
});

test("kv get ok decode", () => {
    const body = Uint8Array.from([1, 2, 0, 0, 0, 65, 66]);
    const value = decodeKvGetOk(body);
    assert.deepEqual(value, new Uint8Array([65, 66]));
});
