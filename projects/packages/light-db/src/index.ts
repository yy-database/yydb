/**
 * `@yydb/light-db`
 *
 * Browser-native YYDB database.
 *
 * Main authoring surface — import {@link Database} from this package, it
 * delegates to `@yydb/yydb-unknown-wasm32` for Rust-backed semantics,
 * schema validation, OPFS persistence and VOS execution.
 *
 * ```ts
 * import { Database } from '@yydb/light-db';
 * const db = await Database.open('app');
 * await db.ensureSchema('...vos source...');
 * await db.put('key', new Uint8Array([1, 2, 3]));
 * ```
 */

// 错误模型
export { LightDbError, LightDbErrorCodes } from './errors';
export type { LightDbErrorOptions } from './errors';

// 作者面 —— WASM-backed Database（主线） + 同构类型合同
export { Database, TransactionHandle, openLightDatabase } from './api';
export type { OpenOptions, ScanRow, ScanOptions, BatchChange, MigrateOptions, SchemaVersion } from './api';
export type { LightDatabase, LightDatabaseOptions } from './api';

// 纯 TS 参考实现（仅测试/回退场景，非生产路径）
export { PureTsDatabase } from './api';

// 内部 schema 编解码（Database 实现使用）
export {
    SCHEMA_KEY,
    RESERVED_KEY_PREFIX,
    documentVersion,
    parseSchemaDocument,
    encodeSchema,
    decodeSchema,
} from './api/schema';
