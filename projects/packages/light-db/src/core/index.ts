/**
 * core 层导出
 *
 * 数据库语义、事务、恢复与单写者合同，不依赖具体存储后端。
 */

export { RecordStore } from "./records";
export { WalLog } from "./wal";
export { CheckpointWriter } from "./checkpoint";
export { RecoveryManager } from "./recovery";
export type { RecoveryOutcome } from "./recovery";
export { acquireWriterLease } from "./lease";
export type { LeaseMode, WriterLease } from "./lease";
export { DatabaseBroadcaster, listenDatabaseEvents } from "./broadcast";
export type { DatabaseEvent, DatabaseEventType } from "./broadcast";
export { StorageEngine, DEFAULT_WAL_THRESHOLD } from "./engine";
export type { CheckpointResult, StorageEngineOptions, StorageEngineStats, TransactionState } from "./engine";
