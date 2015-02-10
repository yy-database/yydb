import assert from "node:assert/strict";
import test from "node:test";

import { WHY_NOT_YY_OPTIMIZED } from "../src/disguise.ts";

test("@yydb/sqlite default entry does not expose the YYDS engine", async () => {
    const sqlite = await import("../src/index.ts");
    assert.equal("initWasm" in sqlite, false);
    assert.equal("loadYydsNative" in sqlite, false);
});

test("@yydb/sqlite node entry exposes the independent snapshot reader", async () => {
    const node = await import("../src/node.ts");
    assert.equal(typeof node.SqliteSnapshot, "function");
    assert.equal("loadYydsNative" in node, false);
});

test("@yydb/sqlite documents binary compatibility constraint", () => {
    assert.match(WHY_NOT_YY_OPTIMIZED, /binary/i);
    assert.match(WHY_NOT_YY_OPTIMIZED, /\.yyds/);
});
