export {
    Yydb,
    TableQuery,
    DdlSession,
    type DdlSessionOptions,
    type YydbCapabilities,
    type YydbListener,
    type YydbLocalState,
    type YydbOptions,
    type YydbStorageOptions,
} from './yydb.js';
export {
    YydbClient,
    type TcpEndpoint,
    type YydbAuth,
    type YydbClientAddress,
    type YydbClientCapabilities,
    type YydbClientOptions,
    type YydbClientState,
    type YydbPoolOptions,
    type YydbTlsOptions,
} from './client.js';
export {
    DatabaseError,
    isDatabaseError,
    type DatabaseErrorExecution,
    type DatabaseErrorInit,
    type DatabaseErrorPhase,
    type DatabaseErrorRetry,
} from './shared/envelope.js';
export {
    Database,
    Statement,
    type DateScalar,
    type DecimalScalar,
    type FileManifestReference,
    type FileScalar,
    type OpenOptions,
    type ParameterDescriptor,
    type PutFileOptions,
    type QueryKind,
    type ScalarValue,
    type SchemaFieldDescriptor,
    type SchemaFieldType,
    type SchemaTableDescriptor,
    type SchemaVersion,
    type SlotType,
    type StatementInfo,
    type TimeScalar,
    type TimestampScalar,
    type UniqueConstraintDescriptor,
} from './api/database.js';
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
export { loadNative } from './native/binding.js';
export * from './shared/vos.js';
