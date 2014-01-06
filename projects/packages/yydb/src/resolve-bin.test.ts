import assert from "node:assert/strict";
import test from "node:test";
import { platformKey } from "./resolve-bin.ts";

test("platformKey maps common hosts", () => {
    assert.equal(platformKey("win32", "x64"), "win32-x64");
    assert.equal(platformKey("darwin", "arm64"), "darwin-arm64");
    assert.equal(platformKey("linux", "arm64"), null);
});
