/**
 * codec 层导出
 *
 * 只处理浏览器可用的 `ArrayBuffer` / `Uint8Array`，不依赖宿主文件系统。
 */

export { crc32, checksumMatches } from "./checksum";
export { BinaryUtils, ByteWriter, ByteReader, ChecksumMismatchError } from "./binary";
export {
    SNAPSHOT_MAGIC,
    SNAPSHOT_VERSION,
    DEFAULT_PAGE_SIZE,
    SNAPSHOT_HEADER_SIZE,
    PAGE_HEADER_SIZE,
    PAGE_CHECKSUM_SIZE,
    RECORD_CHECKSUM_SIZE,
    RecordTooLargeError,
    SnapshotFormatError,
    recordSize,
    encodeHeader,
    decodeHeader,
    encodeRecord,
    encodePage,
    decodePage,
    packRecords,
    encodeSnapshot,
    decodeSnapshot,
} from "./page";
export type { SnapshotHeader, StoredRecord, Page } from "./page";
export {
    WAL_RECORD_HEADER_SIZE,
    WAL_RECORD_CHECKSUM_SIZE,
    WalOperation,
    walRecordSize,
    encodeWalRecord,
    decodeWalRecord,
} from "./record";
export type { WalRecord, WalDecodeResult } from "./record";
