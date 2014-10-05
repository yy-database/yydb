/**
 * WAL 记录编解码
 *
 * 记录布局（小端）：
 * LSN(8) + 事务号(8) + 操作(1) + 键长度(4) + 值长度(4) + 键 + 值 + CRC32(4)。
 */

import { ByteReader, ByteWriter, BinaryUtils } from './binary';

/** WAL 记录头部长度（不含尾部校验和） */
export const WAL_RECORD_HEADER_SIZE = 25;

/** WAL 记录尾部校验和长度 */
export const WAL_RECORD_CHECKSUM_SIZE = 4;

/** WAL 操作类型 */
export enum WalOperation {
    /** 写入或覆盖键 */
    Put = 0x01,
    /** 删除键 */
    Delete = 0x02,
    /** 检查点标记 */
    Checkpoint = 0x10,
}

/** WAL 记录 */
export interface WalRecord {
    /** 日志序列号，单调递增 */
    lsn: bigint;
    /** 事务号，0 表示自动提交 */
    transactionId: bigint;
    /** 操作类型 */
    operation: WalOperation;
    /** 键 */
    key: string;
    /** 值，删除或检查点标记为空 */
    value: Uint8Array | null;
}

/** WAL 解码结果 */
export type WalDecodeResult = { status: 'ok'; record: WalRecord; bytesRead: number } | { status: 'incomplete' };

/**
 * 计算记录编码大小
 * @param key 键
 * @param value 值
 */
export function walRecordSize(key: string, value: Uint8Array | null): number {
    const keyLength = BinaryUtils.byteLength(key);
    const valueLength = value === null ? 0 : value.length;
    return WAL_RECORD_HEADER_SIZE + keyLength + valueLength + WAL_RECORD_CHECKSUM_SIZE;
}

/**
 * 序列化 WAL 记录
 * @param record 记录
 */
export function encodeWalRecord(record: WalRecord): Uint8Array {
    const key = BinaryUtils.encode(record.key);
    const value = record.value ?? new Uint8Array(0);
    const writer = new ByteWriter(walRecordSize(record.key, record.value));

    writer.writeBigUInt64LE(record.lsn);
    writer.writeBigUInt64LE(record.transactionId);
    writer.writeUInt8(record.operation);
    writer.writeUInt32LE(key.length);
    writer.writeUInt32LE(value.length);
    writer.writeBytes(key);
    writer.writeBytes(value);
    writer.writeChecksum(0);

    return writer.data;
}

/**
 * 反序列化 WAL 记录
 *
 * 尾部不完整（进程在写入中途退出）返回 `incomplete`，由恢复流程丢弃该尾部。
 * 校验和不匹配属于真实损坏，会抛出 `ChecksumMismatchError`。
 *
 * @param data 数据
 * @param offset 起始偏移量
 */
export function decodeWalRecord(data: Uint8Array, offset: number = 0): WalDecodeResult {
    const reader = new ByteReader(data, offset);

    if (!reader.hasRemaining(WAL_RECORD_HEADER_SIZE)) {
        return { status: 'incomplete' };
    }

    const start = reader.position;
    const lsn = reader.readBigUInt64LE();
    const transactionId = reader.readBigUInt64LE();
    const operation = reader.readUInt8() as WalOperation;
    const keyLength = reader.readUInt32LE();
    const valueLength = reader.readUInt32LE();

    if (!reader.hasRemaining(keyLength + valueLength + WAL_RECORD_CHECKSUM_SIZE)) {
        return { status: 'incomplete' };
    }

    const key = BinaryUtils.decode(reader.readBytes(keyLength));
    const value = valueLength > 0 ? BinaryUtils.copy(reader.readBytes(valueLength)) : null;
    reader.readAndVerifyChecksum(start);

    return {
        status: 'ok',
        record: { lsn, transactionId, operation, key, value },
        bytesRead: reader.position - start,
    };
}
