import assert from "node:assert/strict";
import test from "node:test";
import { decodeFrame, encodeFrame, encodeKvPut, decodeKvGetOk, MsgType } from "./wire.ts";

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

test("kv get ok decode", () => {
    const body = Uint8Array.from([1, 2, 0, 0, 0, 65, 66]);
    const value = decodeKvGetOk(body);
    assert.deepEqual(value, new Uint8Array([65, 66]));
});
