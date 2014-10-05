/**
 * 崩溃恢复
 *
 * 读取最近一次快照，再按 WAL 顺序重放快照之后的记录。
 */

import { ChecksumMismatchError, decodeSnapshot, SnapshotFormatError } from '../codec';
import type { WalRecord } from '../codec';
import { WalOperation } from '../codec';
import { LightDbError, LightDbErrorCodes } from '../errors';
import type { StorageAdapter } from '../storage/StorageAdapter';
import type { WalLog } from './wal';

/** 恢复结果 */
export interface RecoveryOutcome {
    /** 恢复后的记录 */
    records: Map<string, Uint8Array>;
    /** 已覆盖到的最大 LSN */
    lastLsn: bigint;
    /** 载入的页数量 */
    pagesLoaded: number;
    /** 重放的 WAL 记录数量 */
    recordsReplayed: number;
    /** 丢弃的不完整 WAL 尾字节数 */
    discardedTailBytes: number;
}

/**
 * 恢复管理器
 */
export class RecoveryManager {
    private readonly adapter: StorageAdapter;

    constructor(adapter: StorageAdapter) {
        this.adapter = adapter;
    }

    /**
     * 执行恢复
     * @param wal 已打开的 WAL
     */
    public async recover(wal: WalLog): Promise<RecoveryOutcome> {
        const snapshot = await this.loadSnapshot();
        const { records: walRecords, validBytes, fileSize } = await wal.readAll();

        if (validBytes < fileSize) {
            await wal.truncate(validBytes);
        }

        let recordsReplayed = 0;
        let lastLsn = snapshot.lastSnapshotLsn;

        for (const record of walRecords) {
            if (record.lsn <= snapshot.lastSnapshotLsn) {
                continue;
            }
            this.apply(snapshot.records, record);
            lastLsn = record.lsn > lastLsn ? record.lsn : lastLsn;
            recordsReplayed++;
        }

        return {
            records: snapshot.records,
            lastLsn,
            pagesLoaded: snapshot.pagesLoaded,
            recordsReplayed,
            discardedTailBytes: fileSize - validBytes,
        };
    }

    /**
     * 读取快照，不存在时返回空记录
     */
    private async loadSnapshot(): Promise<{
        records: Map<string, Uint8Array>;
        lastSnapshotLsn: bigint;
        pagesLoaded: number;
    }> {
        if (!(await this.adapter.exists('main'))) {
            return { records: new Map(), lastSnapshotLsn: 0n, pagesLoaded: 0 };
        }

        const handle = await this.adapter.open('main', 'read');
        let data: Uint8Array;
        try {
            const size = await handle.size();
            data = await handle.read(0, size);
        } finally {
            await handle.close();
        }

        try {
            const snapshot = decodeSnapshot(data);
            return {
                records: snapshot.records,
                lastSnapshotLsn: snapshot.header.lastSnapshotLsn,
                pagesLoaded: snapshot.pageCount,
            };
        } catch (error) {
            if (error instanceof SnapshotFormatError || error instanceof ChecksumMismatchError) {
                throw new LightDbError(LightDbErrorCodes.corrupt, 'database snapshot is corrupt', {
                    operation: 'recover',
                    cause: error,
                });
            }
            throw error;
        }
    }

    /**
     * 应用一条 WAL 记录
     * @param records 记录映射
     * @param record WAL 记录
     */
    private apply(records: Map<string, Uint8Array>, record: WalRecord): void {
        switch (record.operation) {
            case WalOperation.Put:
                if (record.value) {
                    records.set(record.key, record.value);
                }
                break;
            case WalOperation.Delete:
                records.delete(record.key);
                break;
            case WalOperation.Checkpoint:
                break;
        }
    }
}
