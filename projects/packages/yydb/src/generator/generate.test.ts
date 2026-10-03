import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, it } from "node:test";

import { generateTypeScript, parseVosSubset, VosParseError } from "../../dist/generator/index.js";

const fixturesDir = path.join(path.dirname(fileURLToPath(import.meta.url)), "fixtures");

function normalizeNewlines(text: string): string {
    return text.replace(/\r\n/g, "\n");
}

describe("parseVosSubset", () => {
    it("parses the setting table fixture", () => {
        const source = fs.readFileSync(path.join(fixturesDir, "setting.vos"), "utf8");
        const ir = parseVosSubset(source, 1);
        assert.equal(ir.tables.length, 1);
        assert.equal(ir.tables[0]?.name, "Setting");
        assert.equal(ir.tables[0]?.fields.length, 3);
        assert.equal(ir.tables[0]?.fields[0]?.isId, true);
    });

    it("rejects tables without @@id", () => {
        assert.throws(
            () =>
                parseVosSubset(
                    "table Broken { name: utf8, }",
                    1,
                ),
            VosParseError,
        );
    });
});

describe("generateTypeScript", () => {
    it("matches the setting.generated.ts snapshot", () => {
        const vos = fs.readFileSync(path.join(fixturesDir, "setting.vos"), "utf8");
        const expected = fs.readFileSync(path.join(fixturesDir, "setting.generated.ts"), "utf8");
        const got = generateTypeScript(vos, 1);
        assert.equal(normalizeNewlines(got), normalizeNewlines(expected));
    });
});
