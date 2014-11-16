/**
 * api 层导出
 *
 * 作者面两条路径：
 *   - {@link BrowserDatabase}  → `database.ts`（纯 TS，无 WASM 依赖）
 *   - {@link Database}          → `wasm-database.ts`（WASM 包装，需要 wasm32 模块）
 *
 * 浏览器测试只想走纯 TS 路径时，import 从 `./database` 来的 `BrowserDatabase` 即可。
 */

export { BrowserDatabase } from './database';
export type { OpenOptions, ScanRow, ScanOptions, BatchChange, MigrateOptions, SchemaVersion } from './database';

export { Database, TransactionHandle, openLightDatabase } from './wasm-database';

export { Transaction } from './transaction';
export type { TransactionState } from './transaction';
export { SCHEMA_KEY, RESERVED_KEY_PREFIX, documentVersion, parseSchemaDocument, encodeSchema, decodeSchema } from './schema';
export type { LightDatabase, LightDatabaseOptions } from './types';
