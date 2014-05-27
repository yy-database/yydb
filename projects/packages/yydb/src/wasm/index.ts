export { Database, type WasmOpenOptions } from './database.js';
export {
    checkSchema,
    createMemorySession,
    initWasm,
    introspectSchema,
    openPersistentSession,
    queryMemory,
    resetWasmBindingForTests,
    yydbVersion,
    type CheckSchemaResult,
    type InitWasmOptions,
    type WasmInitInput,
    type WasmSession,
} from './binding.js';
export { defineMicro, type DefinedMicro, type MicroHandle, type TsMicroDefinitionInput, MicroSessionRegistry } from '../shared/micro.js';
export {
    bool,
    i64,
    nullType,
    text,
    type InferUdfArgs,
    type InferUdfReturn,
    type MicroEffect,
    type MicroInvocationMode,
    type UdfScalarKind,
    type UdfTypeDescriptor,
} from '../shared/udf-types.js';
export type { SchemaVersion } from '../shared/wasm-envelope.js';
