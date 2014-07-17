import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { YydbError, YydbErrorCodes } from '@yydb/yydb-client';
import { parseWasmKvGetResult, parseWasmScalarResult, parseWasmSchemaResult, parseWasmUnitResult } from './wasm-envelope.ts';

describe('wasm-envelope', () => {
    it('parseWasmUnitResult accepts ok payloads', () => {
        parseWasmUnitResult('{"ok":true,"rows":[],"error":null,"code":null}', 'execute');
    });

    it('parseWasmUnitResult rejects error payloads with the engine code', () => {
        assert.throws(
            () => parseWasmUnitResult('{"ok":false,"rows":[],"error":"boom","code":"yydb.schema.invalid"}', 'execute'),
            (error: unknown) => {
                assert.ok(error instanceof YydbError);
                assert.equal(error.code, 'yydb.schema.invalid');
                assert.equal(error.operation, 'execute');
                assert.equal(error.message, 'boom');
                return true;
            },
        );
    });

    it('parseWasmUnitResult reports a stale wasm envelope as a decode failure', () => {
        assert.throws(
            () => parseWasmUnitResult('{"ok":false,"rows":[],"error":"boom"}', 'execute'),
            (error: unknown) => {
                assert.ok(error instanceof YydbError);
                assert.equal(error.code, YydbErrorCodes.clientDecodeFailure);
                return true;
            },
        );
    });

    it('parseWasmSchemaResult returns null schema', () => {
        assert.equal(parseWasmSchemaResult('{"ok":true,"schema":null,"error":null,"code":null}', 'schemaGet'), null);
    });

    it('parseWasmKvGetResult decodes byte arrays', () => {
        const value = parseWasmKvGetResult('{"ok":true,"value":[97,98],"error":null,"code":null}', 'kvGet');
        assert.deepEqual(value, Uint8Array.from([97, 98]));
    });

    it('parseWasmScalarResult decodes scalar payloads', () => {
        assert.equal(parseWasmScalarResult('{"ok":true,"value":42,"error":null,"code":null}', 'callScalar'), 42);
    });
});
