/**
 * api 层导出
 *
 * 主线是 {@link Database}（纯 TS，无 WASM 依赖，与 Node 侧 `@yydb/yydb` 同构）。
 * 需要 WASM 可选后端的宿主显式从 `./wasm-database` 引入 {@link WasmDatabase}。
 */

export { Database } from './database';
export type { OpenOptions, ScanRow, ScanOptions, BatchChange, MigrateOptions, SchemaVersion } from './database';

export { WasmDatabase, TransactionHandle, openLightDatabase } from './wasm-database';

export { Transaction } from './transaction';
export type { TransactionState } from './transaction';
export { SCHEMA_KEY, RESERVED_KEY_PREFIX, documentVersion, parseSchemaDocument, encodeSchema, decodeSchema } from './schema';
export type { LightDatabase, LightDatabaseOptions } from './types';
