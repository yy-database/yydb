/**
 * `@yydb/light-db`
 *
 * 浏览器专用的纯 TypeScript 数据库，与 `@yydb/yydb` 保持同构的用户 API。
 * 公共类型只有 `Uint8Array` 与 `Promise`，不依赖 Node `Buffer`、文件系统或 WASM。
 *
 * @packageDocumentation
 */

// 错误模型
export { LightDbError, LightDbErrorCodes } from "./errors";
export type { LightDbErrorOptions } from "./errors";

// 编解码
export {
    BinaryUtils,
    ByteWriter,
    ByteReader,
    ChecksumMismatchError,
    crc32,
    checksumMatches,
    DEFAULT_PAGE_SIZE,
    SNAPSHOT_MAGIC,
    SNAPSHOT_VERSION,
    SNAPSHOT_HEADER_SIZE,
    PAGE_HEADER_SIZE,
    RecordTooLargeError,
    SnapshotFormatError,
    WalOperation,
    encodeSnapshot,
    decodeSnapshot,
    encodeWalRecord,
    decodeWalRecord,
} from "./codec";
export type { SnapshotHeader, StoredRecord, Page, WalRecord, WalDecodeResult } from "./codec";

// 存储平台边界
export {
    STORAGE_FILES,
    MemoryStorageAdapter,
    MemoryStorageBackend,
    OpfsStorageAdapter,
    OpfsStorageAdapterFactory,
    isOpfsAvailable,
    IndexedDbStorageAdapter,
    IndexedDbStorageAdapterFactory,
    isIndexedDbAvailable,
} from "./storage";
export type {
    StorageAdapter,
    StorageAdapterFactory,
    StorageAdapterKind,
    StorageFile,
    StorageHandle,
    StorageOpenMode,
} from "./storage";

// 数据库语义
export { StorageEngine, DEFAULT_WAL_THRESHOLD, acquireWriterLease, listenDatabaseEvents } from "./core";
export type {
    CheckpointResult,
    LeaseMode,
    RecoveryOutcome,
    StorageEngineOptions,
    StorageEngineStats,
    TransactionState,
    DatabaseEvent,
    DatabaseEventType,
    WriterLease,
} from "./core";

// 作者面
export {
    BrowserDatabase,
    openLightDatabase,
    Transaction,
    SCHEMA_KEY,
    RESERVED_KEY_PREFIX,
    documentVersion,
    parseSchemaDocument,
    encodeSchema,
    decodeSchema,
} from "./api";
export type { LightDatabase, LightDatabaseOptions, SchemaVersion } from "./api";
