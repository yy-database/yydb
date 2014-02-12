import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { defineMicro, MicroSessionRegistry } from "./micro.ts";
import { i64, text } from "./udf-types.ts";

describe("defineMicro", () => {
    it("builds a typed definition without registering", () => {
        const defined = defineMicro({
            name: "math.double",
            version: 1,
            args: [i64()],
            returns: i64(),
            effect: "pure",
            deterministic: true,
            fn(value: number) {
                return value * 2;
            },
        });
        assert.equal(defined.name, "math.double");
        assert.equal(defined.placement, "host");
        assert.equal(defined.fn(21), 42);
    });

    it("rejects non-pure or non-deterministic micros", () => {
        assert.throws(() =>
            defineMicro({
                name: "bad",
                version: 1,
                args: [text()],
                returns: text(),
                effect: "pure",
                deterministic: false,
                fn(value: string) {
                    return value;
                },
            }),
        );
    });
});

describe("MicroSessionRegistry", () => {
    it("registers and invokes scalar micros through opaque handles", () => {
        const registry = new MicroSessionRegistry();
        const defined = defineMicro({
            name: "text.normalize",
            version: 1,
            args: [text()],
            returns: text(),
            effect: "pure",
            deterministic: true,
            fn(value: string) {
                return value.trim().toLowerCase();
            },
        });
        const handle = registry.register(defined);
        assert.equal(handle.name, "text.normalize");
        const result = registry.invokeScalar(handle, ["  Hi  "]);
        assert.equal(result, "hi");
        assert.deepEqual(registry.list(), ["text.normalize"]);
    });

    it("rejects duplicate registration", () => {
        const registry = new MicroSessionRegistry();
        const defined = defineMicro({
            name: "dup",
            version: 1,
            args: [i64()],
            returns: i64(),
            effect: "pure",
            deterministic: true,
            fn(value: number) {
                return value;
            },
        });
        registry.register(defined);
        assert.throws(() => registry.register(defined));
    });
});
