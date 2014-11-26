export { Database, Statement, type OpenOptions, type SchemaVersion, type ParameterDescriptor, type QueryKind, type SlotType, type StatementInfo } from './api/database.js';
export { YydbError, YydbErrorCodes, YydbClientErrorCodes, isYydbError } from './shared/errors.js';
export type { YydbErrorCode, YydbErrorOptions } from './shared/errors.js';
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
