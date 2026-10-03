import assert from 'node:assert/strict';
import test from 'node:test';
import { decodeFrame, encodeFrame, encodeKvPut, decodeKvGetOk, encodeMicroRegister, MsgType } from './wire.ts';

test('frame roundtrip', () => {
    const encoded = encodeFrame({
        msgType: MsgType.Hello,
        flags: 0,
        requestId: 9,
        body: new TextEncoder().encode('x'),
    });
    assert.equal(new TextDecoder().decode(encoded.subarray(0, 4)), 'YYDB');
    assert.equal(new TextDecoder().decode(encoded.subarray(4, 8)), '0000');
    const decoded = decodeFrame(encoded);
    assert.equal(decoded.msgType, MsgType.Hello);
    assert.equal(decoded.requestId, 9);
    assert.equal(decoded.product, 'YYDB');
    assert.equal(new TextDecoder().decode(decoded.body), 'x');
});

test('kv put body encodes lengths', () => {
    const body = encodeKvPut('a', new TextEncoder().encode('bc'));
    assert.ok(body.byteLength >= 4 + 1 + 4 + 2);
});

test('micro host invoke roundtrip codec', async () => {
    const { decodeMicroHostInvoke, encodeMicroHostInvokeOk } = await import('./wire.ts');
    // use inline encode via scalar call helper - import decode only tested via manual body
    const body = new Uint8Array([
        9,
        0,
        0,
        0,
        0,
        0,
        0,
        0, // host_id
        1,
        0,
        0,
        0, // handle_version
        1,
        0,
        0,
        0,
        49, // function_id "1"
        1, // arg count
        3,
        3,
        0,
        0,
        0,
        72,
        105,
        33, // text "Hi!"
    ]);
    const payload = decodeMicroHostInvoke(body);
    assert.equal(payload.hostId, 9);
    assert.equal(payload.functionId, '1');
    assert.equal(payload.args[0], 'Hi!');
    const ok = encodeMicroHostInvokeOk('hi');
    assert.equal(ok.byteLength, 1 + 4 + 2);
});

test('micro register body encodes contract fields', () => {
    const body = encodeMicroRegister({
        hostId: 9,
        handleVersion: 1,
        udfVersion: 2,
        name: 'text.normalize',
        functionId: '3',
        args: ['text'],
        returns: 'text',
        fingerprint: Uint8Array.from({ length: 32 }, (_, index) => index),
    });
    assert.equal(body.byteLength, 8 + 4 + 4 + (4 + 14) + (4 + 1) + 1 + 1 + 1 + 32);
});

test('kv get ok decode', () => {
    const body = Uint8Array.from([1, 2, 0, 0, 0, 65, 66]);
    const value = decodeKvGetOk(body);
    assert.deepEqual(value, new Uint8Array([65, 66]));
});
