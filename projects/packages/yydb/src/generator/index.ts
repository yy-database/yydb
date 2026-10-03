import { emitTypeScript } from './emit-typescript.js';
import { parseVosSubset, VosParseError } from './parse-vos.js';

export type { SchemaField, SchemaTable, ScalarType, TsSchemaIr } from './ir.js';
export { emitTypeScript } from './emit-typescript.js';
export { parseVosSubset, VosParseError } from './parse-vos.js';

/** Generate TypeScript from a VOS document. */
export function generateTypeScript(vosSource: string, schemaVersion: number): string {
    const ir = parseVosSubset(vosSource, schemaVersion);
    return emitTypeScript(ir);
}

export { generateTypeScript as generateTypescript };
