export { Database, type WasmOpenOptions } from './database.js';
export { YydbError, YydbErrorCodes, isRetryableYydbCode, isYydbError } from '@yydb/yydb-client';
export type { YydbErrorCode, YydbErrorOptions } from '@yydb/yydb-client';
export {
    checkSchema,
    createMemorySession,
    initWasm,
    introspectSchema,
    openPersistentSession,
    resetWasmBindingForTests,
    yydbVersion,
    type CheckSchemaResult,
    type InitWasmOptions,
    type WasmInitInput,
    type WasmSession,
} from './binding.js';
export { defineMicro, type DefinedMicro, type MicroHandle, type TsMicroDefinitionInput, MicroSessionRegistry } from '../shared/micro.js';
export {
    type InferUdfArgs,
    type InferUdfReturn,
    type MicroEffect,
    type MicroInvocationMode,
    type UdfScalarDescriptor,
    type UdfScalarKind,
} from '../shared/udf-types.js';
export type { SchemaVersion } from '../shared/wasm-envelope.js';
export * from '../shared/vos.js';
