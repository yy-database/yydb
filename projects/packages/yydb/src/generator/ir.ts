/** Scalar types supported by the MVP emitter (expand with `vos-parser`). */
export type ScalarType = "uuid" | "utf8" | "bool" | "i64" | "f64";

export interface SchemaField {
    readonly name: string;
    readonly ty: ScalarType;
    /** True when this field is declared via `@@id`. */
    readonly isId: boolean;
}

export interface SchemaTable {
    readonly name: string;
    readonly fields: readonly SchemaField[];
}

/** One schema document ready for TypeScript emission. */
export interface TsSchemaIr {
    /** Original VOS source (database truth embedded into generated module). */
    readonly vosSource: string;
    readonly schemaVersion: number;
    readonly tables: readonly SchemaTable[];
}

export function scalarToTs(ty: ScalarType): string {
    switch (ty) {
        case "uuid":
        case "utf8":
            return "string";
        case "bool":
            return "boolean";
        case "i64":
        case "f64":
            return "number";
    }
}
