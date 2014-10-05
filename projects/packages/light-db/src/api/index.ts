/**
 * api 层导出
 */

export { BrowserDatabase, openLightDatabase } from './database';
export { Transaction } from './transaction';
export type { TransactionState } from './transaction';
export { SCHEMA_KEY, RESERVED_KEY_PREFIX, documentVersion, parseSchemaDocument, encodeSchema, decodeSchema } from './schema';
export type { LightDatabase, LightDatabaseOptions, SchemaVersion } from './types';
