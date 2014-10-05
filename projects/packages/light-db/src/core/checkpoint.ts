/**
 * 检查点
 *
 * 把内存记录写成一份新快照，再用适配器的 `rename` 原子发布。
 * 崩溃发生在发布前时旧快照仍然完整，WAL 会在下次打开时重放。
 */

import { encodeSnapshot } from '../codec';
import { LightDbError, LightDbErrorCodes } from '../errors';
import type { StorageAdapter } from '../storage/StorageAdapter';

/**
 * 检查点写入器
 */
export class CheckpointWriter {
    private readonly adapter: StorageAdapter;
    private readonly pageSize: number;

    constructor(adapter: StorageAdapter, pageSize: number) {
        this.adapter = adapter;
        this.pageSize = pageSize;
    }

    /**
     * 写入快照
     * @param records 记录视图
     * @param lsn 快照覆盖到的 LSN
     * @returns 写入字节数
     */
    public async write(records: ReadonlyMap<string, Uint8Array>, lsn: bigint): Promise<number> {
        let data: Uint8Array;
        try {
            data = encodeSnapshot(records, lsn, this.pageSize);
        } catch (error) {
            throw new LightDbError(LightDbErrorCodes.unsupported, 'cannot encode database snapshot', {
                operation: 'checkpoint',
                cause: error,
            });
        }

        const temp = await this.adapter.open('main.tmp', 'readwrite');
        try {
            await temp.truncate(0);
            await temp.write(0, data);
            await temp.sync();
        } finally {
            await temp.close();
        }

        await this.adapter.rename('main.tmp', 'main');
        return data.length;
    }
}
