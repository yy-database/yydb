import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { parseWasmKvGetResult, parseWasmRowsResult, parseWasmSchemaResult, parseWasmUnitResult } from './wasm-envelope.ts';

describe('wasm-envelope', () => {
    it('parseWasmRowsResult accepts ok payloads', () => {
        const result = parseWasmRowsResult('{"ok":true,"rows":[{"id":1}],"error":null}');
        assert.equal(result.rows.length, 1);
    });

    it('parseWasmRowsResult rejects error payloads', () => {
        assert.throws(() => parseWasmRowsResult('{"ok":false,"rows":[],"error":"boom"}'), /boom/);
    });

    it('parseWasmUnitResult accepts ok payloads', () => {
        parseWasmUnitResult('{"ok":true,"rows":[],"error":null}');
    });

    it('parseWasmSchemaResult returns null schema', () => {
        assert.equal(parseWasmSchemaResult('{"ok":true,"schema":null,"error":null}'), null);
    });

    it('parseWasmKvGetResult decodes byte arrays', () => {
        const value = parseWasmKvGetResult('{"ok":true,"value":[97,98],"error":null}');
        assert.deepEqual(value, Uint8Array.from([97, 98]));
    });
});
