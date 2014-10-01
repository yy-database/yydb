/**
 * OPFS 存储适配器
 *
 * 默认持久化后端。优先使用 sync access handle（需 Worker 上下文），
 * 主线程退化为 `createWritable` + `getFile`，两条路径共享同一套逻辑文件语义。
 *
 * 不把 OPFS 目录结构暴露给上层：上层只认识 `main` / `main.tmp` / `wal` / `shm`。
 */

import { LightDbError, LightDbErrorCodes } from "../../errors";
import type { StorageAdapter, StorageAdapterFactory, StorageFile, StorageHandle, StorageOpenMode } from "../StorageAdapter";
import { STORAGE_FILES } from "../StorageAdapter";

/** 逻辑文件在 OPFS 中的物理文件名 */
const PHYSICAL_NAMES: Record<StorageFile, string> = {
    main: "main.snapshot",
    "main.tmp": "main.snapshot.tmp",
    wal: "wal.log",
    shm: "wal.index",
};

/** 文件元信息 */
interface FileLike {
    size: number;
    arrayBuffer(): Promise<ArrayBuffer>;
}

/** 可写流 */
interface WritableLike {
    write(options: { type: "write"; position: number; data: Uint8Array }): Promise<void>;
    truncate(size: number): Promise<void>;
    close(): Promise<void>;
}

/** 同步访问句柄 */
interface SyncAccessHandleLike {
    read(buffer: Uint8Array, options?: { at?: number }): number;
    write(buffer: Uint8Array, options?: { at?: number }): number;
    truncate(size: number): void;
    flush(): void;
    getSize(): number;
    close(): void;
}

/** 文件句柄 */
interface FileHandleLike {
    getFile(): Promise<FileLike>;
    createWritable(options?: { keepExistingData?: boolean }): Promise<WritableLike>;
    createSyncAccessHandle?(): Promise<SyncAccessHandleLike>;
    move?(name: string): Promise<void>;
}

/** 目录句柄 */
interface DirectoryHandleLike {
    getFileHandle(name: string, options?: { create?: boolean }): Promise<FileHandleLike>;
    getDirectoryHandle(name: string, options?: { create?: boolean }): Promise<DirectoryHandleLike>;
    removeEntry(name: string, options?: { recursive?: boolean }): Promise<void>;
}

/** StorageManager 结构化类型 */
interface StorageManagerLike {
    getDirectory(): Promise<DirectoryHandleLike>;
}

/**
 * 判断当前宿主是否提供 OPFS
 */
export function isOpfsAvailable(): boolean {
    const storage = (globalThis as { navigator?: { storage?: StorageManagerLike } }).navigator?.storage;
    return typeof storage?.getDirectory === "function";
}

/**
 * OPFS 适配器工厂
 */
export class OpfsStorageAdapterFactory implements StorageAdapterFactory {
    /** 适配器类型 */
    public readonly kind = "opfs" as const;

    /** 数据持久化 */
    public readonly persistent = true;

    private root: Promise<DirectoryHandleLike> | null = null;

    /**
     * 打开数据库适配器
     * @param name 数据库名
     */
    public async openDatabase(name: string): Promise<StorageAdapter> {
        const root = await this.getRoot();
        const directory = await this.getDirectory(root, encodeURIComponent(name), "openDatabase");
        return new OpfsStorageAdapter(name, directory);
    }

    /**
     * 取得 OPFS 根目录
     */
    private async getRoot(): Promise<DirectoryHandleLike> {
        const storage = (globalThis as { navigator?: { storage?: StorageManagerLike } }).navigator?.storage;
        if (!storage) {
            throw new LightDbError(LightDbErrorCodes.unsupported, "OPFS is not available in this environment", {
                operation: "openDatabase",
            });
        }

        if (!this.root) {
            this.root = storage.getDirectory();
        }

        try {
            return await this.root;
        } catch (error) {
            this.root = null;
            throw new LightDbError(LightDbErrorCodes.io, "failed to open the OPFS root directory", {
                operation: "openDatabase",
                cause: error,
            });
        }
    }

    /**
     * 取得或创建目录
     * @param parent 父目录
     * @param name 目录名
     * @param operation 操作名
     */
    private async getDirectory(parent: DirectoryHandleLike, name: string, operation: string): Promise<DirectoryHandleLike> {
        try {
            return await parent.getDirectoryHandle(name, { create: true });
        } catch (error) {
            throw new LightDbError(LightDbErrorCodes.io, `failed to open OPFS directory "${name}"`, {
                operation,
                cause: error,
            });
        }
    }
}

/**
 * OPFS 存储适配器
 */
export class OpfsStorageAdapter implements StorageAdapter {
    /** 适配器类型 */
    public readonly kind = "opfs" as const;

    /** 数据持久化 */
    public readonly persistent = true;

    private readonly name: string;
    private readonly directory: DirectoryHandleLike;
    private closed: boolean = false;

    constructor(name: string, directory: DirectoryHandleLike) {
        this.name = name;
        this.directory = directory;
    }

    /**
     * 打开逻辑文件
     * @param file 逻辑文件名
     * @param mode 打开模式
     */
    public async open(file: StorageFile, mode: StorageOpenMode = "readwrite"): Promise<StorageHandle> {
        this.assertOpen("storageOpen");
        const physical = PHYSICAL_NAMES[file];

        let handle: FileHandleLike;
        try {
            handle = await this.directory.getFileHandle(physical, { create: mode === "readwrite" });
        } catch (error) {
            throw new LightDbError(LightDbErrorCodes.io, `failed to open OPFS file "${physical}"`, {
                operation: "storageOpen",
                cause: error,
            });
        }

        return new OpfsStorageHandle(physical, handle);
    }

    /**
     * 判断文件是否存在
     * @param file 逻辑文件名
     */
    public async exists(file: StorageFile): Promise<boolean> {
        this.assertOpen("storageExists");
        try {
            await this.directory.getFileHandle(PHYSICAL_NAMES[file]);
            return true;
        } catch {
            return false;
        }
    }

    /**
     * 删除文件
     * @param file 逻辑文件名
     */
    public async delete(file: StorageFile): Promise<void> {
        this.assertOpen("storageDelete");
        try {
            await this.directory.removeEntry(PHYSICAL_NAMES[file]);
        } catch (error) {
            if (isNotFound(error)) {
                return;
            }
            throw new LightDbError(LightDbErrorCodes.io, `failed to delete OPFS file "${PHYSICAL_NAMES[file]}"`, {
                operation: "storageDelete",
                cause: error,
            });
        }
    }

    /**
     * 发布快照：优先 `move`，否则复制后删除
     * @param from 源文件
     * @param to 目标文件
     */
    public async rename(from: StorageFile, to: StorageFile): Promise<void> {
        this.assertOpen("storageRename");
        const fromName = PHYSICAL_NAMES[from];
        const toName = PHYSICAL_NAMES[to];

        try {
            const source = await this.directory.getFileHandle(fromName);
            if (typeof source.move === "function") {
                await source.move(toName);
                return;
            }
        } catch (error) {
            throw new LightDbError(LightDbErrorCodes.io, `failed to rename OPFS file "${fromName}"`, {
                operation: "storageRename",
                cause: error,
            });
        }

        const data = await this.readAll(from);
        const target = await this.open(to, "readwrite");
        try {
            await target.truncate(0);
            await target.write(0, data);
            await target.sync();
        } finally {
            await target.close();
        }
        await this.delete(from);
    }

    /**
     * 列出已存在的文件
     */
    public async list(): Promise<StorageFile[]> {
        this.assertOpen("storageList");
        const present: StorageFile[] = [];
        for (const file of STORAGE_FILES) {
            if (await this.exists(file)) {
                present.push(file);
            }
        }
        return present;
    }

    /** 关闭适配器 */
    public async close(): Promise<void> {
        this.closed = true;
    }

    /**
     * 读取逻辑文件全部内容
     * @param file 逻辑文件名
     */
    private async readAll(file: StorageFile): Promise<Uint8Array> {
        const handle = await this.open(file, "read");
        try {
            return await handle.read(0, await handle.size());
        } finally {
            await handle.close();
        }
    }

    /**
     * 断言适配器未关闭
     * @param operation 操作名
     */
    private assertOpen(operation: string): void {
        if (this.closed) {
            throw new LightDbError(LightDbErrorCodes.sessionClosed, `OPFS database "${this.name}" is closed`, {
                operation,
            });
        }
    }
}

/**
 * OPFS 文件句柄
 */
class OpfsStorageHandle implements StorageHandle {
    private readonly physical: string;
    private readonly handle: FileHandleLike;
    private syncHandle: SyncAccessHandleLike | null = null;
    private syncProbed: boolean = false;
    private closed: boolean = false;

    constructor(physical: string, handle: FileHandleLike) {
        this.physical = physical;
        this.handle = handle;
    }

    /**
     * 读取指定区间
     * @param position 起始位置
     * @param length 期望长度
     */
    public async read(position: number, length: number): Promise<Uint8Array> {
        this.assertOpen("read");
        if (length <= 0) {
            return new Uint8Array(0);
        }

        const sync = await this.getSyncHandle();
        if (sync) {
            const buffer = new Uint8Array(length);
            const bytesRead = sync.read(buffer, { at: position });
            return buffer.subarray(0, bytesRead);
        }

        const file = await this.handle.getFile();
        const data = new Uint8Array(await file.arrayBuffer());
        if (position >= data.length) {
            return new Uint8Array(0);
        }
        return data.subarray(position, Math.min(position + length, data.length));
    }

    /**
     * 写入指定位置
     * @param position 起始位置
     * @param data 数据
     */
    public async write(position: number, data: Uint8Array): Promise<void> {
        this.assertOpen("write");

        const sync = await this.getSyncHandle();
        if (sync) {
            sync.write(data, { at: position });
            return;
        }

        await this.withWritable(async (writable) => {
            await writable.write({ type: "write", position, data });
        }, "write");
    }

    /**
     * 截断文件
     * @param size 目标大小
     */
    public async truncate(size: number): Promise<void> {
        this.assertOpen("truncate");

        const sync = await this.getSyncHandle();
        if (sync) {
            sync.truncate(size);
            return;
        }

        await this.withWritable(async (writable) => {
            await writable.truncate(size);
        }, "truncate");
    }

    /** 落盘 */
    public async sync(): Promise<void> {
        this.assertOpen("sync");
        if (this.syncHandle) {
            this.syncHandle.flush();
        }
        // `createWritable` 路径每次写入都会 close 提交，无需额外 flush。
    }

    /** 当前文件大小 */
    public async size(): Promise<number> {
        this.assertOpen("size");
        if (this.syncHandle) {
            return this.syncHandle.getSize();
        }
        const file = await this.handle.getFile();
        return file.size;
    }

    /** 关闭句柄 */
    public async close(): Promise<void> {
        if (this.closed) {
            return;
        }
        this.closed = true;
        if (this.syncHandle) {
            this.syncHandle.close();
            this.syncHandle = null;
        }
    }

    /**
     * 尝试取得同步访问句柄
     *
     * 主线程没有 `createSyncAccessHandle` 或调用被拒绝时返回 null，退化到可写流。
     */
    private async getSyncHandle(): Promise<SyncAccessHandleLike | null> {
        if (this.syncHandle) {
            return this.syncHandle;
        }

        if (this.syncProbed || typeof this.handle.createSyncAccessHandle !== "function") {
            return null;
        }

        this.syncProbed = true;
        try {
            this.syncHandle = await this.handle.createSyncAccessHandle();
            return this.syncHandle;
        } catch {
            return null;
        }
    }

    /**
     * 使用可写流执行一次提交
     * @param action 写入动作
     * @param operation 操作名
     */
    private async withWritable(action: (writable: WritableLike) => Promise<void>, operation: string): Promise<void> {
        let writable: WritableLike;
        try {
            writable = await this.handle.createWritable({ keepExistingData: true });
        } catch (error) {
            throw new LightDbError(LightDbErrorCodes.io, `failed to open OPFS writable "${this.physical}"`, {
                operation,
                cause: error,
            });
        }

        try {
            await action(writable);
        } catch (error) {
            throw new LightDbError(LightDbErrorCodes.io, `failed to write OPFS file "${this.physical}"`, {
                operation,
                cause: error,
            });
        } finally {
            await writable.close();
        }
    }

    /**
     * 断言句柄未关闭
     * @param operation 操作名
     */
    private assertOpen(operation: string): void {
        if (this.closed) {
            throw new LightDbError(LightDbErrorCodes.sessionClosed, `OPFS handle "${this.physical}" is closed`, {
                operation,
            });
        }
    }
}

/**
 * 判断是否为「文件不存在」错误
 * @param error 错误
 */
function isNotFound(error: unknown): boolean {
    return typeof error === "object" && error !== null && (error as { name?: string }).name === "NotFoundError";
}
