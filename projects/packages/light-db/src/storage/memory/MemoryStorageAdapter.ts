/**
 * 内存存储适配器
 *
 * 用于测试、无持久化需求的一次性会话，以及适配器合同的参考实现。
 */

import { LightDbError, LightDbErrorCodes } from '../../errors';
import type { StorageAdapter, StorageAdapterFactory, StorageFile, StorageHandle, StorageOpenMode } from '../StorageAdapter';
import { STORAGE_FILES } from '../StorageAdapter';

/** 单个数据库的内存目录 */
type MemoryDirectory = Map<StorageFile, Uint8Array>;

/**
 * 内存存储后端
 *
 * 只有共用同一个后端实例才能跨 `openDatabase` 读到上一会话的数据，
 * 测试据此模拟「关闭后重开」。
 */
export class MemoryStorageBackend implements StorageAdapterFactory {
    /** 适配器类型 */
    public readonly kind = 'memory' as const;

    /** 会话结束后数据不保留 */
    public readonly persistent = false;

    private readonly directories: Map<string, MemoryDirectory> = new Map();

    /**
     * 打开数据库适配器
     * @param name 数据库名
     */
    public async openDatabase(name: string): Promise<StorageAdapter> {
        let directory = this.directories.get(name);
        if (!directory) {
            directory = new Map();
            this.directories.set(name, directory);
        }
        return new MemoryStorageAdapter(name, directory);
    }

    /** 清空所有数据库 */
    public reset(): void {
        this.directories.clear();
    }
}

/**
 * 内存存储适配器
 */
export class MemoryStorageAdapter implements StorageAdapter {
    /** 适配器类型 */
    public readonly kind = 'memory' as const;

    /** 会话结束后数据不保留 */
    public readonly persistent = false;

    private readonly name: string;
    private readonly directory: MemoryDirectory;
    private closed: boolean = false;

    constructor(name: string, directory: MemoryDirectory) {
        this.name = name;
        this.directory = directory;
    }

    /**
     * 打开逻辑文件
     * @param file 逻辑文件名
     * @param mode 打开模式
     */
    public async open(file: StorageFile, mode: StorageOpenMode = 'readwrite'): Promise<StorageHandle> {
        this.assertOpen('open');

        const existing = this.directory.get(file);
        if (!existing && mode === 'read') {
            throw new LightDbError(LightDbErrorCodes.io, `storage file "${file}" does not exist in "${this.name}"`, {
                operation: 'storageOpen',
            });
        }

        if (!existing) {
            this.directory.set(file, new Uint8Array(0));
        }

        return new MemoryStorageHandle(this.directory, file);
    }

    /**
     * 判断文件是否存在
     * @param file 逻辑文件名
     */
    public async exists(file: StorageFile): Promise<boolean> {
        this.assertOpen('exists');
        return this.directory.has(file);
    }

    /**
     * 删除文件
     * @param file 逻辑文件名
     */
    public async delete(file: StorageFile): Promise<void> {
        this.assertOpen('delete');
        this.directory.delete(file);
    }

    /**
     * 原子重命名
     * @param from 源文件
     * @param to 目标文件
     */
    public async rename(from: StorageFile, to: StorageFile): Promise<void> {
        this.assertOpen('rename');
        const data = this.directory.get(from);
        if (!data) {
            throw new LightDbError(LightDbErrorCodes.io, `cannot rename missing storage file "${from}"`, {
                operation: 'storageRename',
            });
        }
        this.directory.set(to, data);
        this.directory.delete(from);
    }

    /**
     * 列出已存在的文件
     */
    public async list(): Promise<StorageFile[]> {
        this.assertOpen('list');
        return STORAGE_FILES.filter((file) => this.directory.has(file));
    }

    /** 关闭适配器 */
    public async close(): Promise<void> {
        this.closed = true;
    }

    /**
     * 断言适配器未关闭
     * @param operation 操作名
     */
    private assertOpen(operation: string): void {
        if (this.closed) {
            throw new LightDbError(LightDbErrorCodes.sessionClosed, 'storage adapter is closed', { operation });
        }
    }
}

/**
 * 内存文件句柄
 */
class MemoryStorageHandle implements StorageHandle {
    private readonly directory: MemoryDirectory;
    private readonly file: StorageFile;
    private closed: boolean = false;

    constructor(directory: MemoryDirectory, file: StorageFile) {
        this.directory = directory;
        this.file = file;
    }

    /**
     * 读取指定区间
     * @param position 起始位置
     * @param length 期望长度
     */
    public async read(position: number, length: number): Promise<Uint8Array> {
        this.assertOpen('read');
        const data = this.readData();
        if (position >= data.length) {
            return new Uint8Array(0);
        }
        return data.slice(position, Math.min(position + length, data.length));
    }

    /**
     * 写入指定位置
     * @param position 起始位置
     * @param data 数据
     */
    public async write(position: number, data: Uint8Array): Promise<void> {
        this.assertOpen('write');
        const current = this.readData();
        const required = position + data.length;
        const next = current.length >= required ? current : grow(current, required);
        next.set(data, position);
        this.directory.set(this.file, next);
    }

    /**
     * 截断文件
     * @param size 目标大小
     */
    public async truncate(size: number): Promise<void> {
        this.assertOpen('truncate');
        const current = this.readData();
        if (size <= current.length) {
            this.directory.set(this.file, current.slice(0, size));
            return;
        }
        this.directory.set(this.file, grow(current, size));
    }

    /** 内存写入无需落盘 */
    public async sync(): Promise<void> {
        this.assertOpen('sync');
    }

    /** 当前文件大小 */
    public async size(): Promise<number> {
        this.assertOpen('size');
        return this.readData().length;
    }

    /** 关闭句柄 */
    public async close(): Promise<void> {
        this.closed = true;
    }

    /**
     * 读取当前文件内容
     */
    private readData(): Uint8Array {
        return this.directory.get(this.file) ?? new Uint8Array(0);
    }

    /**
     * 断言句柄未关闭
     * @param operation 操作名
     */
    private assertOpen(operation: string): void {
        if (this.closed) {
            throw new LightDbError(LightDbErrorCodes.sessionClosed, 'storage handle is closed', { operation });
        }
    }
}

/**
 * 扩展到指定大小，新增区间填零
 * @param current 当前内容
 * @param size 目标大小
 */
function grow(current: Uint8Array, size: number): Uint8Array {
    const next = new Uint8Array(size);
    next.set(current, 0);
    return next;
}
