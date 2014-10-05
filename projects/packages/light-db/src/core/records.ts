/**
 * 记录存储
 *
 * 数据库在内存中的键值视图。值在写入与读取时都做一次复制，
 * 避免调用方持有内部字节数组造成校验和与快照不一致。
 */

import { BinaryUtils } from '../codec';

/**
 * 内存记录存储
 */
export class RecordStore {
    private records: Map<string, Uint8Array> = new Map();

    /**
     * 读取值
     * @param key 键
     */
    public get(key: string): Uint8Array | null {
        const value = this.records.get(key);
        return value ? BinaryUtils.copy(value) : null;
    }

    /**
     * 是否存在键
     * @param key 键
     */
    public has(key: string): boolean {
        return this.records.has(key);
    }

    /**
     * 写入值
     * @param key 键
     * @param value 值
     */
    public put(key: string, value: Uint8Array): void {
        this.records.set(key, BinaryUtils.copy(value));
    }

    /**
     * 删除键
     * @param key 键
     */
    public delete(key: string): boolean {
        return this.records.delete(key);
    }

    /** 记录数量 */
    public get size(): number {
        return this.records.size;
    }

    /**
     * 快照当前记录
     *
     * 返回只读视图，供检查点打包使用。
     */
    public snapshot(): ReadonlyMap<string, Uint8Array> {
        return this.records;
    }

    /**
     * 用恢复结果替换全部记录
     * @param records 记录映射
     */
    public restore(records: ReadonlyMap<string, Uint8Array>): void {
        this.records = new Map();
        for (const [key, value] of records) {
            this.records.set(key, BinaryUtils.copy(value));
        }
    }

    /** 清空所有记录 */
    public clear(): void {
        this.records.clear();
    }
}
