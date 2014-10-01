/**
 * WAL 日志
 *
 * 只通过 `StorageAdapter` 访问平台，记录格式由 `codec/record` 决定。
 */

import { ChecksumMismatchError, decodeWalRecord, encodeWalRecord } from "../codec";
import type { WalRecord } from "../codec";
import { LightDbError, LightDbErrorCodes } from "../errors";
import type { StorageAdapter, StorageHandle } from "../storage/StorageAdapter";

/**
 * 预写日志
 */
export class WalLog {
    private readonly adapter: StorageAdapter;
    private handle: StorageHandle | null = null;
    private offset: number = 0;

    constructor(adapter: StorageAdapter) {
        this.adapter = adapter;
    }

    /** 当前日志字节数 */
    public get byteSize(): number {
        return this.offset;
    }

    /**
     * 打开日志，定位到末尾
     */
    public async open(): Promise<void> {
        this.handle = await this.adapter.open("wal", "readwrite");
        this.offset = await this.handle.size();
    }

    /**
     * 追加记录并等待落盘
     * @param records 记录
     */
    public async append(records: readonly WalRecord[]): Promise<number> {
        if (records.length === 0) {
            return 0;
        }

        const handle = this.requireHandle("walAppend");
        const encoded = records.map((record) => encodeWalRecord(record));
        const total = encoded.reduce((sum, part) => sum + part.length, 0);

        const buffer = new Uint8Array(total);
        let position = 0;
        for (const part of encoded) {
            buffer.set(part, position);
            position += part.length;
        }

        await handle.write(this.offset, buffer);
        await handle.sync();
        this.offset += total;
        return total;
    }

    /**
     * 读取全部完整记录
     *
     * 末尾不完整的记录（写入过程中断）会被丢弃，`validBytes` 指向最后一个完整记录之后。
     */
    public async readAll(): Promise<{ records: WalRecord[]; validBytes: number; fileSize: number }> {
        const handle = this.requireHandle("walRead");
        const size = await handle.size();
        if (size === 0) {
            return { records: [], validBytes: 0, fileSize: 0 };
        }

        const data = await handle.read(0, size);
        const records: WalRecord[] = [];
        let offset = 0;

        while (offset < data.length) {
            try {
                const decoded = decodeWalRecord(data, offset);
                if (decoded.status === "incomplete") {
                    break;
                }
                records.push(decoded.record);
                offset += decoded.bytesRead;
            } catch (error) {
                if (error instanceof ChecksumMismatchError) {
                    throw new LightDbError(LightDbErrorCodes.corrupt, `WAL record is corrupt at offset ${offset}`, {
                        operation: "walRead",
                        cause: error,
                    });
                }
                throw error;
            }
        }

        return { records, validBytes: offset, fileSize: size };
    }

    /**
     * 截断日志
     * @param size 目标字节数，默认 0
     */
    public async truncate(size: number = 0): Promise<void> {
        const handle = this.requireHandle("walTruncate");
        await handle.truncate(size);
        await handle.sync();
        this.offset = size;
    }

    /** 关闭日志 */
    public async close(): Promise<void> {
        if (this.handle) {
            await this.handle.close();
            this.handle = null;
        }
    }

    /**
     * 获取已打开的句柄
     * @param operation 操作名
     */
    private requireHandle(operation: string): StorageHandle {
        if (!this.handle) {
            throw new LightDbError(LightDbErrorCodes.sessionClosed, "WAL is not open", { operation });
        }
        return this.handle;
    }
}
