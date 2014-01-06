/** Browser-safe entry (WebSocket). For Node TCP use `@yydb/yydb-client/node`. */

export { Client } from "./client.js";
export type { ConnectOptions } from "./client.js";
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
    decodeSchemaGetOk,
    decodeKvGetOk,
} from "./wire.js";
export type { Frame, MsgTypeCode, ProductMagic, SchemaVersion } from "./wire.js";
