/**
 * IndexedDB 存储适配器
 *
 * 兼容后端：宿主没有 OPFS，或需要在同一浏览器配置里复用既有存储时使用。
 * 它只实现 `StorageAdapter` 合同，事务、恢复与检查点语义仍由 `core` 决定，
 * 两套后端不各自实现一套数据库语义。
 *
 * IndexedDB 没有按位置随机写的原生能力，因此句柄在内存中维护整份文件内容，
 * `sync()` 与 `close()` 负责把它写回对象仓库。这牺牲了 OPFS 那样的细粒度
 * 原子性，属于兼容后端的已知取舍。
 */

import { LightDbError, LightDbErrorCodes } from "../../errors";
import type { StorageAdapter, StorageAdapterFactory, StorageFile, StorageHandle, StorageOpenMode } from "../StorageAdapter";
import { STORAGE_FILES } from "../StorageAdapter";

/** 对象仓库名 */
const STORE_NAME = "files";

/** IndexedDB 数据库版本 */
const DB_VERSION = 1;

/** 数据库名前缀，避免与宿主自己的对象仓库冲突 */
const DATABASE_PREFIX = "yydb:light:";

/** IndexedDB 请求的结构化视图 */
interface IdbRequestLike<T> {
    result: T;
    error: unknown;
    onsuccess: (() => void) | null;
    onerror: (() => void) | null;
}

/** IndexedDB 打开请求的结构化视图 */
interface IdbOpenRequestLike extends IdbRequestLike<IdbDatabaseLike> {
    onupgradeneeded: (() => void) | null;
    onblocked: (() => void) | null;
}

/** 对象仓库的结构化视图 */
interface IdbObjectStoreLike {
    get(key: string): IdbRequestLike<ArrayBuffer | undefined>;
    put(value: ArrayBuffer, key: string): IdbRequestLike<unknown>;
    delete(key: string): IdbRequestLike<unknown>;
    getAllKeys(): IdbRequestLike<string[]>;
}

/** 事务的结构化视图 */
interface IdbTransactionLike {
    error: unknown;
    oncomplete: (() => void) | null;
    onerror: (() => void) | null;
    onabort: (() => void) | null;
    objectStore(name: string): IdbObjectStoreLike;
    abort(): void;
}

/** 数据库的结构化视图 */
interface IdbDatabaseLike {
    objectStoreNames: { contains(name: string): boolean };
    createObjectStore(name: string): IdbObjectStoreLike;
    transaction(name: string, mode?: "readonly" | "readwrite"): IdbTransactionLike;
    close(): void;
}

/** IndexedDB 工厂的结构化视图 */
interface IdbFactoryLike {
    open(name: string, version?: number): IdbOpenRequestLike;
}

/**
 * 判断当前宿主是否提供 IndexedDB
 */
export function isIndexedDbAvailable(): boolean {
    const factory = (globalThis as { indexedDB?: IdbFactoryLike }).indexedDB;
    return typeof factory?.open === "function";
}

/**
 * IndexedDB 适配器工厂
 */
export class IndexedDbStorageAdapterFactory implements StorageAdapterFactory {
    /** 适配器类型 */
    public readonly kind = "indexeddb" as const;

    /** 数据持久化 */
    public readonly persistent = true;

    /**
     * 打开数据库适配器
     * @param name 数据库名
     */
    public async openDatabase(name: string): Promise<StorageAdapter> {
        const database = await openIdbDatabase(`${DATABASE_PREFIX}${name}`, "openDatabase");
        return new IndexedDbStorageAdapter(name, database);
    }
}

/**
 * IndexedDB 存储适配器
 */
export class IndexedDbStorageAdapter implements StorageAdapter {
    /** 适配器类型 */
    public readonly kind = "indexeddb" as const;

    /** 数据持久化 */
    public readonly persistent = true;

    private readonly name: string;
    private readonly db: IdbDatabaseLike;
    private closed: boolean = false;

    constructor(name: string, db: IdbDatabaseLike) {
        this.name = name;
        this.db = db;
    }

    /**
     * 打开逻辑文件
     * @param file 逻辑文件名
     * @param mode 打开模式
     */
    public async open(file: StorageFile, mode: StorageOpenMode = "readwrite"): Promise<StorageHandle> {
        this.assertOpen("storageOpen");

        const data = await this.readFile(file);
        if (data === null && mode === "read") {
            throw new LightDbError(LightDbErrorCodes.io, `storage file "${file}" does not exist in "${this.name}"`, {
                operation: "storageOpen",
            });
        }

        return new IndexedDbStorageHandle(this, file, data ?? new Uint8Array(0), mode === "read");
    }

    /**
     * 判断文件是否存在
     * @param file 逻辑文件名
     */
    public async exists(file: StorageFile): Promise<boolean> {
        this.assertOpen("storageExists");
        return (await this.readFile(file)) !== null;
    }

    /**
     * 删除文件，不存在时视为成功
     * @param file 逻辑文件名
     */
    public async delete(file: StorageFile): Promise<void> {
        this.assertOpen("storageDelete");
        await this.writeTransaction("storageDelete", (store) => {
            store.delete(file);
        });
    }

    /**
     * 原子重命名：同一事务内写入目标并删除源
     * @param from 源文件
     * @param to 目标文件
     */
    public async rename(from: StorageFile, to: StorageFile): Promise<void> {
        this.assertOpen("storageRename");
        const data = await this.readFile(from);
        if (data === null) {
            throw new LightDbError(LightDbErrorCodes.io, `cannot rename missing storage file "${from}"`, {
                operation: "storageRename",
            });
        }

        await this.writeTransaction("storageRename", (store) => {
            store.put(toArrayBuffer(data), to);
            store.delete(from);
        });
    }

    /**
     * 列出已存在的文件
     */
    public async list(): Promise<StorageFile[]> {
        this.assertOpen("storageList");
        const keys = await this.readTransaction("storageList", (store) =>
            requestResult(store.getAllKeys(), "storageList", `failed to list IndexedDB database "${this.name}"`),
        );
        return STORAGE_FILES.filter((file) => keys.includes(file));
    }

    /** 关闭适配器 */
    public async close(): Promise<void> {
        if (this.closed) {
            return;
        }
        this.closed = true;
        try {
            this.db.close();
        } catch (error) {
            throw toStorageError(error, "storageClose", `failed to close IndexedDB database "${this.name}"`);
        }
    }

    /**
     * 写回逻辑文件内容
     * @param file 逻辑文件名
     * @param data 文件内容
     */
    public async persist(file: StorageFile, data: Uint8Array): Promise<void> {
        this.assertOpen("write");
        await this.writeTransaction("write", (store) => {
            store.put(toArrayBuffer(data), file);
        });
    }

    /**
     * 读取逻辑文件内容
     * @param file 逻辑文件名
     * @returns 内容，文件不存在时返回 null
     */
    private readFile(file: StorageFile): Promise<Uint8Array | null> {
        return this.readTransaction("read", async (store) => {
            const buffer = await requestResult(store.get(file), "read", `failed to read storage file "${file}"`);
            return buffer === undefined ? null : new Uint8Array(buffer);
        });
    }

    /**
     * 执行只读事务
     * @param operation 操作名
     * @param action 请求构造
     */
    private async readTransaction<T>(operation: string, action: (store: IdbObjectStoreLike) => Promise<T> | T): Promise<T> {
        const tx = this.beginTransaction("readonly", operation);
        const done = awaitTransaction(tx, operation, `IndexedDB transaction failed for "${this.name}"`);
        const result = await action(tx.objectStore(STORE_NAME));
        await done;
        return result;
    }

    /**
     * 执行读写事务
     * @param operation 操作名
     * @param action 请求构造
     */
    private async writeTransaction(operation: string, action: (store: IdbObjectStoreLike) => void): Promise<void> {
        const tx = this.beginTransaction("readwrite", operation);
        const done = awaitTransaction(tx, operation, `IndexedDB transaction failed for "${this.name}"`);
        try {
            action(tx.objectStore(STORE_NAME));
        } catch (error) {
            try {
                tx.abort();
            } catch {
                // 事务已经结束，保留原始错误
            }
            await done.catch(() => undefined);
            throw toStorageError(error, operation, `failed to write IndexedDB database "${this.name}"`);
        }
        await done;
    }

    /**
     * 创建事务
     * @param mode 事务模式
     * @param operation 操作名
     */
    private beginTransaction(mode: "readonly" | "readwrite", operation: string): IdbTransactionLike {
        try {
            return this.db.transaction(STORE_NAME, mode);
        } catch (error) {
            throw toStorageError(error, operation, `failed to start IndexedDB transaction for "${this.name}"`);
        }
    }

    /**
     * 断言适配器未关闭
     * @param operation 操作名
     */
    private assertOpen(operation: string): void {
        if (this.closed) {
            throw new LightDbError(LightDbErrorCodes.sessionClosed, `IndexedDB database "${this.name}" is closed`, {
                operation,
            });
        }
    }
}

/**
 * IndexedDB 文件句柄
 *
 * 内容常驻内存，`sync()` 与 `close()` 写回对象仓库。
 */
class IndexedDbStorageHandle implements StorageHandle {
    private readonly adapter: IndexedDbStorageAdapter;
    private readonly file: StorageFile;
    private readonly readOnly: boolean;
    private data: Uint8Array;
    private dirty: boolean = false;
    private closed: boolean = false;

    constructor(adapter: IndexedDbStorageAdapter, file: StorageFile, data: Uint8Array, readOnly: boolean) {
        this.adapter = adapter;
        this.file = file;
        this.data = data;
        this.readOnly = readOnly;
    }

    /**
     * 读取指定区间
     * @param position 起始位置
     * @param length 期望长度
     */
    public async read(position: number, length: number): Promise<Uint8Array> {
        this.assertOpen("read");
        if (position >= this.data.length || length <= 0) {
            return new Uint8Array(0);
        }
        return this.data.slice(position, Math.min(position + length, this.data.length));
    }

    /**
     * 写入指定位置
     * @param position 起始位置
     * @param data 数据
     */
    public async write(position: number, data: Uint8Array): Promise<void> {
        this.assertOpen("write");
        const required = position + data.length;
        if (this.data.length < required) {
            const next = new Uint8Array(required);
            next.set(this.data, 0);
            this.data = next;
        }
        this.data.set(data, position);
        this.dirty = true;
    }

    /**
     * 截断文件
     * @param size 目标大小
     */
    public async truncate(size: number): Promise<void> {
        this.assertOpen("truncate");
        if (size <= this.data.length) {
            this.data = this.data.slice(0, size);
        } else {
            const next = new Uint8Array(size);
            next.set(this.data, 0);
            this.data = next;
        }
        this.dirty = true;
    }

    /** 写回对象仓库 */
    public async sync(): Promise<void> {
        this.assertOpen("sync");
        if (!this.dirty || this.readOnly) {
            return;
        }
        await this.adapter.persist(this.file, this.data);
        this.dirty = false;
    }

    /** 当前文件大小 */
    public async size(): Promise<number> {
        this.assertOpen("size");
        return this.data.length;
    }

    /** 关闭句柄 */
    public async close(): Promise<void> {
        if (this.closed) {
            return;
        }
        await this.sync();
        this.closed = true;
    }

    /**
     * 断言句柄未关闭
     * @param operation 操作名
     */
    private assertOpen(operation: string): void {
        if (this.closed) {
            throw new LightDbError(LightDbErrorCodes.sessionClosed, `IndexedDB handle for "${this.file}" is closed`, {
                operation,
            });
        }
    }
}

/**
 * 打开 IndexedDB 数据库
 * @param name 物理数据库名
 * @param operation 操作名
 */
function openIdbDatabase(name: string, operation: string): Promise<IdbDatabaseLike> {
    return new Promise((resolve, reject) => {
        const factory = (globalThis as { indexedDB?: IdbFactoryLike }).indexedDB;
        if (!factory) {
            reject(
                new LightDbError(LightDbErrorCodes.unsupported, "IndexedDB is not available in this environment", {
                    operation,
                }),
            );
            return;
        }

        let request: IdbOpenRequestLike;
        try {
            request = factory.open(name, DB_VERSION);
        } catch (error) {
            reject(toStorageError(error, operation, `failed to open IndexedDB database "${name}"`));
            return;
        }

        request.onupgradeneeded = () => {
            const database = request.result;
            if (!database.objectStoreNames.contains(STORE_NAME)) {
                database.createObjectStore(STORE_NAME);
            }
        };
        request.onsuccess = () => {
            resolve(request.result);
        };
        request.onerror = () => {
            reject(toStorageError(request.error, operation, `failed to open IndexedDB database "${name}"`));
        };
        request.onblocked = () => {
            reject(
                new LightDbError(LightDbErrorCodes.io, `IndexedDB database "${name}" is blocked by another connection`, {
                    operation,
                }),
            );
        };
    });
}

/**
 * 等待请求完成
 * @param request 请求
 * @param operation 操作名
 * @param message 失败描述
 */
function requestResult<T>(request: IdbRequestLike<T>, operation: string, message: string): Promise<T> {
    return new Promise((resolve, reject) => {
        request.onsuccess = () => {
            resolve(request.result);
        };
        request.onerror = () => {
            reject(toStorageError(request.error, operation, message));
        };
    });
}

/**
 * 等待事务提交
 * @param tx 事务
 * @param operation 操作名
 * @param message 失败描述
 */
function awaitTransaction(tx: IdbTransactionLike, operation: string, message: string): Promise<void> {
    return new Promise((resolve, reject) => {
        tx.oncomplete = () => {
            resolve();
        };
        tx.onerror = () => {
            reject(toStorageError(tx.error, operation, message));
        };
        tx.onabort = () => {
            reject(toStorageError(tx.error, operation, message));
        };
    });
}

/**
 * 复制为独立的 `ArrayBuffer`
 * @param data 数据
 */
function toArrayBuffer(data: Uint8Array): ArrayBuffer {
    return data.slice().buffer as ArrayBuffer;
}

/**
 * 转换为稳定存储错误
 * @param error 原始错误
 * @param operation 操作名
 * @param message 失败描述
 */
function toStorageError(error: unknown, operation: string, message: string): LightDbError {
    const name = typeof error === "object" && error !== null ? (error as { name?: string }).name : undefined;
    const code = name === "QuotaExceededError" ? LightDbErrorCodes.quotaExceeded : LightDbErrorCodes.io;
    return new LightDbError(code, message, { operation, cause: error });
}
