import type { SqliteRecordValue } from "@yydb/sqlite";

import type { ReferenceValue } from "./manifest.ts";

/** Compares one @yydb/sqlite decoded column against upstream SQLite manifest truth. */
export function matchesReference(column: SqliteRecordValue, expected: ReferenceValue): boolean {
    switch (expected.kind) {
        case "null":
            return column.kind === "null";
        case "integer":
            return column.kind === "integer" && column.valueInteger === expected.value;
        case "real":
            return column.kind === "real"
                && column.valueReal !== undefined
                && realBits(column.valueReal) === expected.bits;
        case "text":
            return column.kind === "text" && column.valueText === expected.value;
        case "blob":
            return column.kind === "blob" && expected.value.equals(column.valueBlob ?? Buffer.alloc(0));
        default:
            return false;
    }
}

function realBits(value: number): bigint {
    const buffer = Buffer.alloc(8);
    buffer.writeDoubleBE(value);
    return buffer.readBigUInt64BE();
}
