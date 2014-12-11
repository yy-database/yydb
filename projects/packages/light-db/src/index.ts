/**
 * `@yydb/light-db`
 *
 * Browser-native YYDB database — a thin TypeScript facade over
 * `@yydb/yydb-unknown-wasm32`, which ships the Rust semantic core
 * compiled to WebAssembly. VOS schema validation, transaction
 * semantics, OPFS persistence and UDF execution all live in WASM.
 *
 * ```ts
 * import { Database } from '@yydb/light-db';
 * const db = await Database.open('app');
 * await db.ensureSchema('// @yydb-schema-version: 1\nUser { ... }');
 * await db.put('User:1', new Uint8Array([1, 2, 3]));
 * const row = await db.get('User:1');
 * await db.close();
 * ```
 */

// 错误模型
export { LightDbError, LightDbErrorCodes } from './errors';
export type { LightDbErrorOptions } from './errors';

// 唯一作者面 —— WASM-backed Database + 稳定类型合同
export { Database, TransactionHandle, openLightDatabase } from './api';
export type {
    BatchChange,
    LightDatabase,
    MigrateOptions,
    OpenOptions,
    ScanOptions,
    ScanRow,
    SchemaVersion,
} from './api';
