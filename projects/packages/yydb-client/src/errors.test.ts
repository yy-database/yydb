import assert from 'node:assert/strict';
import test from 'node:test';
import { YydbError, YydbErrorCodes, isRetryableYydbCode, isYydbError } from './errors.ts';

test('YydbError carries code, operation and retryable flag', () => {
    const error = new YydbError(YydbErrorCodes.schemaInvalid, 'VOS schema: boom', {
        operation: 'schemaEnsure',
    });
    assert.equal(error.name, 'YydbError');
    assert.equal(error.code, 'yydb.schema.invalid');
    assert.equal(error.operation, 'schemaEnsure');
    assert.equal(error.retryable, false);
    assert.ok(isYydbError(error));
});

test('YydbError defaults operation and keeps a wrapped cause', () => {
    const cause = new Error('socket');
    const error = new YydbError(YydbErrorCodes.clientTransportFailure, 'connect: transport failed', {
        cause,
    });
    assert.equal(error.operation, 'unknown');
    assert.equal(error.retryable, true);
    assert.equal(error.cause, cause);
});

test('retryable codes are limited to races and transport hiccups', () => {
    assert.equal(isRetryableYydbCode(YydbErrorCodes.casConflict), true);
    assert.equal(isRetryableYydbCode(YydbErrorCodes.leaseUnavailable), true);
    assert.equal(isRetryableYydbCode(YydbErrorCodes.clientTransportFailure), true);
    assert.equal(isRetryableYydbCode(YydbErrorCodes.schemaConflict), false);
    assert.equal(isRetryableYydbCode(YydbErrorCodes.sessionClosed), false);
    assert.equal(isRetryableYydbCode(YydbErrorCodes.unsupported), false);
});

test('unknown engine codes are accepted as-is', () => {
    const error = new YydbError('yydb.future.code', 'from a newer engine');
    assert.equal(error.code, 'yydb.future.code');
    assert.equal(error.retryable, false);
});
