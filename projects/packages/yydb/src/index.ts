export { Database, type OpenOptions } from './api/database.js';
export { YydbError, YydbErrorCodes, isRetryableYydbCode, isYydbError } from '@yydb/yydb-client';
export type { YydbErrorCode, YydbErrorOptions } from '@yydb/yydb-client';
export { defineMicro, type DefinedMicro, type MicroHandle, type TsMicroDefinitionInput, MicroSessionRegistry } from './shared/micro.js';
export {
    type InferUdfArgs,
    type InferUdfReturn,
    type MicroEffect,
    type MicroInvocationMode,
    type UdfScalarDescriptor,
    type UdfScalarKind,
} from './shared/udf-types.js';
export { nativeBindingName, platformKey, resolveYydbBinary, resolveYydbCli } from './native/resolve.js';
export * from './shared/vos.js';
