import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { v } from '@voml/vos';
import { defineMicro, MicroSessionRegistry } from './micro.ts';

describe('defineMicro', () => {
    it('builds a typed definition without registering', () => {
        const defined = defineMicro({
            name: 'math.double',
            version: 1,
            args: [v.i64()],
            returns: v.i64(),
            effect: 'pure',
            deterministic: true,
            fn(value: bigint) {
                return value * 2n;
            },
        });
        assert.equal(defined.name, 'math.double');
        assert.equal(defined.placement, 'host');
        assert.equal(defined.fn(21n), 42n);
    });

    it('rejects non-pure or non-deterministic micros', () => {
        assert.throws(() =>
            defineMicro({
                name: 'bad',
                version: 1,
                args: [v.utf8()],
                returns: v.utf8(),
                effect: 'pure',
                deterministic: false,
                fn(value: string) {
                    return value;
                },
            }),
        );
    });

    it('rejects descriptors outside the VOS Phase-1 subset', () => {
        assert.throws(
            () =>
                defineMicro({
                    name: 'bad.type',
                    version: 1,
                    args: [v.utf16() as unknown as Parameters<typeof defineMicro>[0]['args'][number]],
                    returns: v.utf8(),
                    effect: 'pure',
                    deterministic: true,
                    fn(value: string) {
                        return value;
                    },
                }),
            /Phase-1 micro subset/,
        );
    });
});

describe('MicroSessionRegistry', () => {
    it('registers and invokes scalar micros through opaque handles', () => {
        const registry = new MicroSessionRegistry();
        const defined = defineMicro({
            name: 'text.normalize',
            version: 1,
            args: [v.utf8()],
            returns: v.utf8(),
            effect: 'pure',
            deterministic: true,
            fn(value: string) {
                return value.trim().toLowerCase();
            },
        });
        const handle = registry.register(defined);
        assert.equal(handle.name, 'text.normalize');
        const result = registry.invokeScalar(handle, ['  Hi  ']);
        assert.equal(result, 'hi');
        assert.deepEqual(registry.list(), ['text.normalize']);
    });

    it('carries i64 across the wire number path as a VOS i64', () => {
        const registry = new MicroSessionRegistry();
        const handle = registry.register(
            defineMicro({
                name: 'math.double',
                version: 1,
                args: [v.i64()],
                returns: v.i64(),
                effect: 'pure',
                deterministic: true,
                fn(value: bigint) {
                    return value * 2n;
                },
            }),
        );
        assert.equal(registry.invokeFromWire({ hostId: handle.hostId, handleVersion: 1, functionId: handle.functionId, args: [21] }), 42);
    });

    it('rejects duplicate registration', () => {
        const registry = new MicroSessionRegistry();
        const defined = defineMicro({
            name: 'dup',
            version: 1,
            args: [v.i64()],
            returns: v.i64(),
            effect: 'pure',
            deterministic: true,
            fn(value: bigint) {
                return value;
            },
        });
        registry.register(defined);
        assert.throws(() => registry.register(defined));
    });
});
