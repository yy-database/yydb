/**
 * api 层导出
 *
 * 主线是 {@link Database}（WASM 后端，基于 `@yydb/yydb-unknown-wasm32` 的
 * Rust 语义核心）。与 Node 侧 `@yydb/yydb` 保持同构的用户 API 和返回语义。
 *
 * 纯 TS {@link PureTsDatabase} 仅作测试/参考——不参与生产路径。
 */

// 主线：WASM Database
export { Database, TransactionHandle, openLightDatabase } from './wasm-database';

// 纯 TS 参考实现（非生产路径）
export { PureTsDatabase } from './database';

// 稳定类型合同（与 Node @yydb/yydb 对齐）
export type { OpenOptions, ScanRow, ScanOptions, BatchChange, MigrateOptions, SchemaVersion } from './database';
export type { LightDatabase, LightDatabaseOptions } from './types';

// 内部 schema 编解码
export { SCHEMA_KEY, RESERVED_KEY_PREFIX, documentVersion, parseSchemaDocument, encodeSchema, decodeSchema } from './schema';
