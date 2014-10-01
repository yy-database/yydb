/**
 * 页与快照编解码
 *
 * 快照由一个固定长度头部和若干固定长度页组成，全部基于 `Uint8Array`。
 */

import { ByteReader, ByteWriter, BinaryUtils, ChecksumMismatchError } from "./binary";
import { crc32 } from "./checksum";

/** 快照魔数：`LIGH` */
export const SNAPSHOT_MAGIC = 0x4c494748;

/** 快照格式版本 */
export const SNAPSHOT_VERSION = 1;

/** 默认页大小：4KB */
export const DEFAULT_PAGE_SIZE = 4096;

/** 快照头部长度：魔数(4) + 版本(4) + 页大小(4) + 最近快照 LSN(8) + 保留(16) */
export const SNAPSHOT_HEADER_SIZE = 36;

/** 页头部长度：页号(4) + 记录数(2) + 空闲空间(2) */
export const PAGE_HEADER_SIZE = 8;

/** 页尾校验和长度 */
export const PAGE_CHECKSUM_SIZE = 4;

/** 记录尾校验和长度 */
export const RECORD_CHECKSUM_SIZE = 4;

/** 快照头部 */
export interface SnapshotHeader {
    /** 魔数 */
    magic: number;
    /** 格式版本 */
    version: number;
    /** 页大小 */
    pageSize: number;
    /** 最近一次快照覆盖到的 LSN */
    lastSnapshotLsn: bigint;
}

/** 持久化记录 */
export interface StoredRecord {
    /** 键 */
    key: string;
    /** 值 */
    value: Uint8Array;
}

/** 页 */
export interface Page {
    /** 页号 */
    pageId: number;
    /** 页内记录 */
    records: StoredRecord[];
}

/** 单条记录超出页容量 */
export class RecordTooLargeError extends Error {
    /** 记录所需字节数 */
    public readonly size: number;

    /** 页可用字节数 */
    public readonly capacity: number;

    constructor(size: number, capacity: number) {
        super(`Record needs ${size} bytes but a page only has ${capacity}`);
        this.name = "RecordTooLargeError";
        this.size = size;
        this.capacity = capacity;
        Object.setPrototypeOf(this, RecordTooLargeError.prototype);
    }
}

/** 快照格式错误 */
export class SnapshotFormatError extends Error {
    /** 失败原因 */
    public readonly reason: string;

    constructor(reason: string) {
        super(`Snapshot format error: ${reason}`);
        this.name = "SnapshotFormatError";
        this.reason = reason;
        Object.setPrototypeOf(this, SnapshotFormatError.prototype);
    }
}

/**
 * 计算记录编码后的字节数
 * @param key 键
 * @param value 值
 */
export function recordSize(key: string, value: Uint8Array): number {
    return 4 + BinaryUtils.byteLength(key) + 4 + value.length + RECORD_CHECKSUM_SIZE;
}

/**
 * 序列化快照头部
 * @param header 头部
 */
export function encodeHeader(header: SnapshotHeader): Uint8Array {
    const writer = new ByteWriter(SNAPSHOT_HEADER_SIZE);
    writer.writeUInt32LE(header.magic);
    writer.writeUInt32LE(header.version);
    writer.writeUInt32LE(header.pageSize);
    writer.writeBigUInt64LE(header.lastSnapshotLsn);
    writer.writeBytes(BinaryUtils.alloc(16));
    return writer.data;
}

/**
 * 反序列化快照头部
 * @param data 数据
 */
export function decodeHeader(data: Uint8Array): SnapshotHeader {
    if (data.length < SNAPSHOT_HEADER_SIZE) {
        throw new SnapshotFormatError(`header needs ${SNAPSHOT_HEADER_SIZE} bytes, got ${data.length}`);
    }

    const reader = new ByteReader(data);
    const magic = reader.readUInt32LE();
    const version = reader.readUInt32LE();
    const pageSize = reader.readUInt32LE();
    const lastSnapshotLsn = reader.readBigUInt64LE();

    if (magic !== SNAPSHOT_MAGIC) {
        throw new SnapshotFormatError(`unexpected magic ${magic}`);
    }

    if (pageSize <= 0) {
        throw new SnapshotFormatError(`invalid page size ${pageSize}`);
    }

    return { magic, version, pageSize, lastSnapshotLsn };
}

/**
 * 序列化单条记录
 * @param record 记录
 */
export function encodeRecord(record: StoredRecord): Uint8Array {
    const writer = new ByteWriter(recordSize(record.key, record.value));
    writer.writeString(record.key);
    writer.writeBytesWithLength(record.value);
    writer.writeChecksum(0);
    return writer.data;
}

/**
 * 序列化页
 * @param page 页
 * @param pageSize 页大小
 */
export function encodePage(page: Page, pageSize: number = DEFAULT_PAGE_SIZE): Uint8Array {
    const capacity = pageSize - PAGE_HEADER_SIZE - PAGE_CHECKSUM_SIZE;
    const payload: Uint8Array[] = [];
    let payloadLength = 0;

    for (const record of page.records) {
        const encoded = encodeRecord(record);
        payload.push(encoded);
        payloadLength += encoded.length;
    }

    if (payloadLength > capacity) {
        throw new RecordTooLargeError(payloadLength, capacity);
    }

    const writer = new ByteWriter(pageSize);
    writer.writeUInt32LE(page.pageId);
    writer.writeUInt16LE(page.records.length);
    writer.writeUInt16LE(capacity - payloadLength);
    for (const part of payload) {
        writer.writeBytes(part);
    }
    writer.writeBytes(BinaryUtils.alloc(pageSize - PAGE_CHECKSUM_SIZE - writer.position));
    writer.writeChecksum(0);
    return writer.data;
}

/**
 * 反序列化页
 * @param data 页数据
 * @param pageId 期望页号
 */
export function decodePage(data: Uint8Array, pageId: number): Page {
    if (data.length < PAGE_HEADER_SIZE + PAGE_CHECKSUM_SIZE) {
        throw new SnapshotFormatError(`page ${pageId} is shorter than its header`);
    }

    const checksumView = new DataView(data.buffer, data.byteOffset + data.length - PAGE_CHECKSUM_SIZE, PAGE_CHECKSUM_SIZE);
    const expectedChecksum = checksumView.getUint32(0, true);
    const actualChecksum = crc32(data, 0, data.length - PAGE_CHECKSUM_SIZE);
    if (actualChecksum !== expectedChecksum) {
        throw new ChecksumMismatchError(expectedChecksum, actualChecksum, 0);
    }

    const reader = new ByteReader(data);
    const readPageId = reader.readUInt32LE();
    const recordCount = reader.readUInt16LE();
    reader.readUInt16LE();

    if (readPageId !== pageId) {
        throw new SnapshotFormatError(`expected page ${pageId}, got ${readPageId}`);
    }

    const records: StoredRecord[] = [];
    for (let i = 0; i < recordCount; i++) {
        const start = reader.position;
        const key = reader.readString();
        const value = BinaryUtils.copy(reader.readBytesWithLength());
        reader.readAndVerifyChecksum(start);
        records.push({ key, value });
    }

    return { pageId, records };
}

/**
 * 将记录打包成页
 * @param records 记录映射
 * @param pageSize 页大小
 */
export function packRecords(records: ReadonlyMap<string, Uint8Array>, pageSize: number = DEFAULT_PAGE_SIZE): Page[] {
    const capacity = pageSize - PAGE_HEADER_SIZE - PAGE_CHECKSUM_SIZE;
    const pages: Page[] = [];
    let current: Page = { pageId: 0, records: [] };
    let used = 0;

    for (const [key, value] of records) {
        const size = recordSize(key, value);
        if (size > capacity) {
            throw new RecordTooLargeError(size, capacity);
        }

        if (current.records.length > 0 && used + size > capacity) {
            pages.push(current);
            current = { pageId: pages.length, records: [] };
            used = 0;
        }

        current.records.push({ key, value });
        used += size;
    }

    if (current.records.length > 0) {
        pages.push(current);
    }

    return pages;
}

/**
 * 序列化完整快照
 * @param records 记录映射
 * @param lastSnapshotLsn 最近快照 LSN
 * @param pageSize 页大小
 */
export function encodeSnapshot(
    records: ReadonlyMap<string, Uint8Array>,
    lastSnapshotLsn: bigint,
    pageSize: number = DEFAULT_PAGE_SIZE,
): Uint8Array {
    const header = encodeHeader({
        magic: SNAPSHOT_MAGIC,
        version: SNAPSHOT_VERSION,
        pageSize,
        lastSnapshotLsn,
    });

    const pages = packRecords(records, pageSize);
    const parts: Uint8Array[] = [header];
    for (const page of pages) {
        parts.push(encodePage(page, pageSize));
    }

    return BinaryUtils.concat(parts);
}

/**
 * 反序列化完整快照
 * @param data 数据
 */
export function decodeSnapshot(data: Uint8Array): {
    header: SnapshotHeader;
    records: Map<string, Uint8Array>;
    pageCount: number;
} {
    const header = decodeHeader(data);
    const pageCount = Math.floor((data.length - SNAPSHOT_HEADER_SIZE) / header.pageSize);
    const records = new Map<string, Uint8Array>();

    for (let pageId = 0; pageId < pageCount; pageId++) {
        const start = SNAPSHOT_HEADER_SIZE + pageId * header.pageSize;
        const page = decodePage(data.subarray(start, start + header.pageSize), pageId);
        for (const record of page.records) {
            records.set(record.key, record.value);
        }
    }

    return { header, records, pageCount };
}
