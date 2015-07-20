import assert from "node:assert/strict";
import test from "node:test";

import { Client, encodeErrorBody, isYydbError, MsgType, YydbClientErrorCodes, YydbError, type Frame, type Transport } from "../dist/index.js";
import { normalizeTcpHostPort, normalizeWsUrl } from "../src/transport.ts";

/**
 * Client protocol usage over a scripted transport: every request maps to its
 * expected reply, ids advance monotonically, and protocol violations surface
 * as stable `yydb.client.*` codes.
 */

interface Scripted {
    transport: Transport;
    sent: Frame[];
    hostHandler: unknown;
}

function scriptedTransport(respond: (frame: Frame) => Frame): Scripted {
    const sent: Frame[] = [];
    const scripted: Scripted = {
        transport: {
            send: async (frame) => {
                sent.push(frame);
                return respond(frame);
            },
            close() {},
            setMicroHostHandler(handler: unknown) {
                scripted.hostHandler = handler;
            },
        },
        sent,
        hostHandler: undefined,
    };
    return scripted;
}

function ok(msgType: number, body: Uint8Array = new Uint8Array()) {
    return (frame: Frame): Frame => ({ msgType, flags: 0, requestId: frame.requestId, body });
}

function u32(value: number): number[] {
    return [value & 0xff, (value >>> 8) & 0xff, (value >>> 16) & 0xff, (value >>> 24) & 0xff];
}

function prefixed(text: string): number[] {
    const bytes = Array.from(new TextEncoder().encode(text));
    return [...u32(bytes.length), ...bytes];
}

test("hello and info return the server's text payloads", async () => {
    const scripted = scriptedTransport(ok(MsgType.HelloOk, new TextEncoder().encode("0.0.18")));
    const client = Client.fromTransport("127.0.0.1:7700", scripted.transport);
    assert.equal(await client.hello(), "0.0.18");
    assert.equal(await client.serverVersion(), "0.0.18");
    assert.equal(scripted.sent[0].msgType, MsgType.Hello);
    assert.equal(scripted.sent[1].msgType, MsgType.Hello);

    const infoClient = Client.fromTransport("127.0.0.1:7700", scriptedTransport(ok(MsgType.InfoOk, new TextEncoder().encode("path=mem"))).transport);
    assert.equal(await infoClient.info(), "path=mem");
});

test("schema get decodes present and absent envelopes from the wire", async () => {
    const document = "table T {\n    @@t_id: i64,\n}";
    const present = Uint8Array.from([1, ...u32(4), ...prefixed(document)]);
    const client = Client.fromTransport("127.0.0.1:7700", scriptedTransport(ok(MsgType.SchemaGetOk, present)).transport);
    assert.deepEqual(await client.getSchema(), { version: 4, document });

    const absent = Client.fromTransport("127.0.0.1:7700", scriptedTransport(ok(MsgType.SchemaGetOk, Uint8Array.from([0]))).transport);
    assert.equal(await absent.getSchema(), null);
});

test("ensure schema ships the version and document bytes verbatim", async () => {
    const scripted = scriptedTransport(ok(MsgType.SchemaEnsureOk));
    const client = Client.fromTransport("127.0.0.1:7700", scripted.transport);
    await client.ensureSchema(2, "table T {\n    @@t_id: i64,\n}");
    const body = scripted.sent[0].body;
    assert.deepEqual(Array.from(body.subarray(0, 4)), u32(2));
    assert.equal(new TextDecoder().decode(body.subarray(8)), "table T {\n    @@t_id: i64,\n}");
});

test("kv put and get move octets through the request and reply bodies", async () => {
    const scripted = scriptedTransport(ok(MsgType.KvPutOk));
    const client = Client.fromTransport("127.0.0.1:7700", scripted.transport);
    await client.put("k", Uint8Array.from([0, 0x80, 0xff]));
    assert.equal(scripted.sent[0].msgType, MsgType.KvPut);

    const octets = Uint8Array.from([0x00, 0x80, 0xff]);
    const getClient = Client.fromTransport(
        "127.0.0.1:7700",
        scriptedTransport(ok(MsgType.KvGetOk, Uint8Array.from([1, ...u32(octets.length), ...octets]))).transport,
    );
    assert.deepEqual(await getClient.get("k"), octets);

    const missing = Client.fromTransport("127.0.0.1:7700", scriptedTransport(ok(MsgType.KvGetOk, Uint8Array.from([0]))).transport);
    assert.equal(await missing.get("k"), null);
});

test("micro register and scalar call complete their request-reply pairs", async () => {
    const scripted = scriptedTransport(ok(MsgType.MicroRegisterOk));
    const client = Client.fromTransport("127.0.0.1:7700", scripted.transport);
    await client.registerMicro({
        hostId: 1,
        handleVersion: 1,
        udfVersion: 1,
        name: "demo.identity",
        functionId: "1",
        args: ["i64"],
        returns: "i64",
        fingerprint: new Uint8Array(32),
    });
    assert.equal(scripted.sent[0].msgType, MsgType.MicroRegister);

    const i64Ok = Uint8Array.from([2, 42, 0, 0, 0, 0, 0, 0, 0]);
    const callClient = Client.fromTransport("127.0.0.1:7700", scriptedTransport(ok(MsgType.ScalarCallOk, i64Ok)).transport);
    assert.equal(await callClient.callScalar("demo.double", 1, [21]), 42);
    assert.equal(scripted.sent.length, 1, "the call client is independent of the register client");

    const nullClient = Client.fromTransport("127.0.0.1:7700", scriptedTransport(ok(MsgType.ScalarCallOk, Uint8Array.from([0]))).transport);
    assert.equal(await nullClient.callScalar("demo.nope", 1, []), null);
});

test("request ids advance monotonically within one client", async () => {
    const scripted = scriptedTransport(ok(MsgType.KvPutOk));
    const client = Client.fromTransport("127.0.0.1:7700", scripted.transport);
    await client.put("a", "1");
    await client.put("b", "2");
    await client.put("c", "3");
    assert.deepEqual(scripted.sent.map((frame) => frame.requestId), [1, 2, 3]);
});

test("a reply with the wrong type reports `yydb.client.unexpected_response`", async () => {
    const client = Client.fromTransport("127.0.0.1:7700", scriptedTransport(ok(MsgType.KvGetOk)).transport);
    await assert.rejects(client.info(), (error: unknown) => {
        assert.ok(isYydbError(error));
        assert.equal(error.code, YydbClientErrorCodes.clientUnexpectedResponse);
        assert.equal(error.operation, "info");
        assert.match(error.message, /unexpected wire response type/);
        return true;
    });
});

test("engine errors keep their code and pick up the request's operation label", async () => {
    const client = Client.fromTransport(
        "127.0.0.1:7700",
        scriptedTransport((frame) => ({
            msgType: MsgType.Error,
            flags: 0,
            requestId: frame.requestId,
            body: encodeErrorBody({ code: "yydb.unique.violation", message: "unique constraint violated" }),
        })).transport,
    );
    await assert.rejects(client.get("k"), (error: unknown) => {
        assert.ok(error instanceof YydbError);
        assert.equal(error.code, "yydb.unique.violation");
        assert.equal(error.operation, "kvGet");
        return true;
    });
});

test("connecting without an endpoint is refused before any transport opens", async () => {
    await assert.rejects(Client.connect({ endpoint: "  " }), (error: unknown) => {
        assert.ok(isYydbError(error));
        assert.equal(error.code, YydbClientErrorCodes.clientInvalidEndpoint);
        assert.equal(error.operation, "connect");
        return true;
    });
});

test("the micro host handler is forwarded to the transport", () => {
    const scripted = scriptedTransport(ok(MsgType.HelloOk));
    const client = Client.fromTransport("127.0.0.1:7700", scripted.transport);
    const handler = () => null;
    client.setMicroHostHandler(handler);
    assert.equal(scripted.hostHandler, handler);
    client.setMicroHostHandler(undefined);
    assert.equal(scripted.hostHandler, undefined);
});

test("endpoint spellings normalize to the wire-equivalent forms", () => {
    assert.equal(normalizeWsUrl("127.0.0.1:7700"), "ws://127.0.0.1:7700/wire");
    assert.equal(normalizeWsUrl("ws://host:1"), "ws://host:1/wire");
    assert.equal(normalizeWsUrl("ws://host:1/wire"), "ws://host:1/wire");
    assert.equal(normalizeWsUrl("http://host:2/"), "ws://host:2/wire");
    assert.equal(normalizeWsUrl("tcp://host:3"), "ws://host:3/wire");
    assert.equal(normalizeTcpHostPort("ws://127.0.0.1:7700/wire"), "127.0.0.1:7700");
    assert.equal(normalizeTcpHostPort("127.0.0.1:7700"), "127.0.0.1:7700");
});
