/**
 * LightDB API 层
 * 提供用户直接使用的接口层
 *
 * @packageDocumentation
 */

export {
    LightDBOptions,
    CollectionCreateOptions,
    TransactionContext,
    TransactionCollection,
    TransactionCallback,
    TransactionOptions,
    TransactionState,
    TransactionResult,
    DatabaseStats,
    DatabaseConfig,
    ILightDB,
    DefineCollectionSchema,
    CollectionSchema,
} from "./types";

export {
    TransactionError,
    Transaction,
    TransactionManager,
} from "./Transaction";

export { LightDB, defineCollectionSchema, openDatabase } from "./LightDB";
