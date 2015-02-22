// 相对 `.js` 说明符只在 tsc 产物里可解析，因此该文件针对 `dist` 公共入口测试。
import assert from 'node:assert/strict';
import test from 'node:test';
import { Client, encodeErrorBody, isYydbError, MsgType, YydbError, YydbErrorCodes, type Frame, type Transport } from '../dist';

function scriptedTransport(respond: (frame: Frame) => Frame): Transport {
    return {
        send: async (frame) => respond(frame),
        close() {},
        setMicroHostHandler() {},
    };
}

test('engine error frame becomes a YydbError with the engine code', async () => {
    const client = Client.fromTransport(
        '127.0.0.1:7700',
        scriptedTransport((frame) => ({
            msgType: MsgType.Error,
            flags: 0,
            requestId: frame.requestId,
            body: encodeErrorBody({
                code: 'yydb.schema.invalid',
                message: 'VOS schema: boom',
            }),
        })),
    );

    await assert.rejects(client.ensureSchema(1, 'table {'), (error: unknown) => {
        assert.ok(isYydbError(error));
        assert.equal(error.code, 'yydb.schema.invalid');
        assert.equal(error.operation, 'schemaEnsure');
        assert.equal(error.message, 'VOS schema: boom');
        return true;
    });
});

test('a malformed engine error frame reports a client decode failure', async () => {
    const client = Client.fromTransport(
        '127.0.0.1:7700',
        scriptedTransport((frame) => ({
            msgType: MsgType.Error,
            flags: 0,
            requestId: frame.requestId,
            body: Uint8Array.from([4, 0, 0, 0]),
        })),
    );

    await assert.rejects(client.info(), (error: unknown) => {
        assert.ok(error instanceof YydbError);
        assert.equal(error.code, YydbErrorCodes.clientDecodeFailure);
        assert.equal(error.operation, 'info');
        return true;
    });
});

test('mismatched request ids report a client protocol failure', async () => {
    const client = Client.fromTransport(
        '127.0.0.1:7700',
        scriptedTransport((frame) => ({
            msgType: MsgType.KvGetOk,
            flags: 0,
            requestId: frame.requestId + 1,
            body: Uint8Array.from([0]),
        })),
    );

    await assert.rejects(client.get('missing'), (error: unknown) => {
        assert.ok(error instanceof YydbError);
        assert.equal(error.code, YydbErrorCodes.clientRequestIdMismatch);
        return true;
    });
});

test('operations after close fail with yydb.session.closed and close is idempotent', async () => {
    const client = Client.fromTransport(
        '127.0.0.1:7700',
        scriptedTransport((frame) => ({
            msgType: MsgType.KvGetOk,
            flags: 0,
            requestId: frame.requestId,
            body: Uint8Array.from([0]),
        })),
    );

    client.close();
    client.close();
    assert.equal(client.isClosed(), true);

    await assert.rejects(client.get('k'), (error: unknown) => {
        assert.ok(error instanceof YydbError);
        assert.equal(error.code, YydbErrorCodes.sessionClosed);
        assert.equal(error.operation, 'kvGet');
        return true;
    });
});

test('transport rejection keeps the cause and is retryable', async () => {
    const cause = new Error('socket closed');
    const client = Client.fromTransport('127.0.0.1:7700', {
        send: async () => {
            throw cause;
        },
        close() {},
        setMicroHostHandler() {},
    });

    await assert.rejects(client.put('k', 'v'), (error: unknown) => {
        assert.ok(error instanceof YydbError);
        assert.equal(error.code, YydbErrorCodes.clientTransportFailure);
        assert.equal(error.retryable, true);
        assert.equal(error.cause, cause);
        return true;
    });
});
