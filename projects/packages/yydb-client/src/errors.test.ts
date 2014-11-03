import assert from 'node:assert/strict';
import test from 'node:test';
import { YydbError, YydbErrorCodes, isYydbError } from './errors.ts';

test('YydbError carries code, operation and retryable flag', () => {
    const error = new YydbError(YydbErrorCodes.schemaInvalid, 'VOS schema: boom', {
        operation: 'schemaEnsure',
    });
    assert.equal(error.name, 'YydbError');
    assert.equal(error.code, 'yydb.schema.invalid');
    assert.equal(error.operation, 'schemaEnsure');
    assert.equal(error.retryable, false); // default conservative
    assert.ok(isYydbError(error));
});

test('YydbError respects explicit retryable option and keeps wrapped cause', () => {
    const cause = new Error('socket');
    const error = new YydbError(YydbErrorCodes.clientTransportFailure, 'connect: transport failed', {
        operation: 'connect',
        retryable: true,
        cause,
    });
    assert.equal(error.operation, 'connect');
    assert.equal(error.retryable, true);
    assert.equal(error.cause, cause);
});

test('fromDiagnosticJson trusts Rust authoritative retryable', () => {
    // Engine errors come through N-API / wire with the full envelope.
    const fromEngine = YydbError.fromDiagnosticJson(JSON.stringify({
        code: 'yydb.lease.unavailable',
        operation: 'lease',
        retryable: true,
        message: 'lease already held',
    }));
    assert.ok(fromEngine);
    assert.equal(fromEngine!.code, 'yydb.lease.unavailable');
    assert.equal(fromEngine!.retryable, true);
    assert.equal(fromEngine!.operation, 'lease');

    // WASM envelope nests the diagnostic under { ok, error }.
    const fromWasm = YydbError.fromDiagnosticJson(JSON.stringify({
        ok: false,
        error: {
            code: 'yydb.schema.invalid',
            operation: 'schema',
            retryable: false,
            message: 'duplicate field "x"',
            params: { field: 'x' },
        },
    }));
    assert.ok(fromWasm);
    assert.equal(fromWasm!.retryable, false);
    assert.deepEqual(fromWasm!.params, { field: 'x' });
});

test('unknown engine codes are accepted as-is', () => {
    const error = new YydbError('yydb.future.code', 'from a newer engine');
    assert.equal(error.code, 'yydb.future.code');
    assert.equal(error.retryable, false); // default conservative when no envelope
});
