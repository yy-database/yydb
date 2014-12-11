/**
 * api 层导出
 *
 * 唯一的生产路径：{@link Database}（WASM-backed，委托给
 * `@yydb/yydb-unknown-wasm32` 的 Rust 语义核心）。
 */

export { Database, TransactionHandle, openLightDatabase } from './wasm-database';

export {
    SCHEMA_KEY,
    RESERVED_KEY_PREFIX,
    documentVersion,
    parseSchemaDocument,
    encodeSchema,
    decodeSchema,
} from './schema';

export type {
    BatchChange,
    LightDatabase,
    MigrateOptions,
    OpenOptions,
    ScanOptions,
    ScanRow,
    SchemaVersion,
} from './types';
