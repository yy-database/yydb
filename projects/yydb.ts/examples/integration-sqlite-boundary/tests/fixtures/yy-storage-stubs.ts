import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { TempFixture } from "./shared.ts";

const PAGE_SIZE = 4096;

/** Minimal on-disk headers that must not be accepted as SQLite format-3. */
const YY_STORAGE_STUBS: Readonly<Record<string, Buffer>> = {
    ".yydb": Buffer.concat([Buffer.from("YYDB\x01", "binary"), Buffer.alloc(PAGE_SIZE - 5)]),
    ".yyds": Buffer.concat([Buffer.from("YYDS\x01", "binary"), Buffer.alloc(PAGE_SIZE - 5)]),
    ".yykv": Buffer.concat([Buffer.from("YYKV\x01", "binary"), Buffer.alloc(PAGE_SIZE - 5)]),
    ".yydx": Buffer.concat([Buffer.from("YYDX\x01", "binary"), Buffer.alloc(PAGE_SIZE - 5)]),
};

export function writeYyStorageStub(extension: keyof typeof YY_STORAGE_STUBS): TempFixture & { bytes: Buffer } {
    const bytes = YY_STORAGE_STUBS[extension];
    const directory = mkdtempSync(join(tmpdir(), "yydb-sqlite-negative-"));
    const path = join(directory, `storage${extension}`);
    writeFileSync(path, bytes);
    return { directory, path, bytes };
}

export const YY_STORAGE_EXTENSIONS = Object.keys(YY_STORAGE_STUBS) as (keyof typeof YY_STORAGE_STUBS)[];
