/** Browser-safe entry (WebSocket). For Node TCP use `@yydb/yydb-client/node`. */

export { Client } from './client.js';
export type { ConnectOptions } from './client.js';
export {
    MAGIC,
    MAGIC_YYDB,
    MAGIC_YYDS,
    WIRE_VERSION,
    WIRE_VERSION_NEXT,
    HEADER_LEN,
    MsgType,
    encodeFrame,
    decodeFrame,
    encodeSchemaEnsure,
    encodeKvGet,
    encodeKvPut,
    encodeMicroRegister,
    encodeScalarCall,
    encodeErrorBody,
    decodeErrorBody,
    decodeScalarCallOk,
    decodeSchemaGetOk,
    decodeKvGetOk,
} from './wire.js';
export type {
    Frame,
    MicroRegisterPayload,
    MsgTypeCode,
    ProductMagic,
    SchemaVersion,
    WireErrorBody,
    WireUdfScalarKind,
    WireUdfScalarValue,
} from './wire.js';
export { YydbError, YydbErrorCodes, isRetryableYydbCode, isYydbError } from './errors.js';
export type { YydbErrorCode, YydbErrorOptions } from './errors.js';
export type { MicroHostHandler, Transport } from './transport.js';
