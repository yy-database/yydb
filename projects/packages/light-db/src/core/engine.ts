/**
 * 存储引擎
 *
 * 只处理数据库语义：记录、WAL、检查点、恢复与事务。
 * 所有平台能力都经 `StorageAdapter`，本层不接触浏览器或 Node 文件 API。
 */

import { BinaryUtils, DEFAULT_PAGE_SIZE, WalOperation } from "../codec";
import { LightDbError, LightDbErrorCodes } from "../errors";
import type { StorageAdapter, StorageAdapterFactory } from "../storage/StorageAdapter";
import { DatabaseBroadcaster } from "./broadcast";
import { CheckpointWriter } from "./checkpoint";
import { acquireWriterLease } from "./lease";
import type { LeaseMode, WriterLease } from "./lease";
import { RecordStore } from "./records";
import { RecoveryManager } from "./recovery";
import { WalLog } from "./wal";

/** 默认 WAL 阈值：16MB */
export const DEFAULT_WAL_THRESHOLD = 16 * 1024 * 1024;

/** 存储引擎选项 */
export interface StorageEngineOptions {
    /** 页大小，默认 4KB */
    pageSize?: number;
    /** WAL 字节阈值，超过后自动检查点，默认 16MB */
    walThreshold?: number;
    /** 是否自动检查点，默认 true */
    autoCheckpoint?: boolean;
    /** 写租约模式，默认 `web-locks` */
    lease?: LeaseMode;
}

/** 事务状态 */
export type TransactionState = "idle" | "active";

/** 检查点结果 */
export interface CheckpointResult {
    /** 检查点覆盖到的 LSN */
    lsn: bigint;
    /** 写入字节数 */
    bytesWritten: number;
    /** WAL 截断前的字节数 */
    walBytesReleased: number;
}

/** 引擎统计 */
export interface StorageEngineStats {
    /** 数据库名 */
    name: string;
    /** 当前 LSN */
    lsn: bigint;
    /** 记录数量 */
    recordCount: number;
    /** WAL 当前字节数 */
    walBytes: number;
    /** 页大小 */
    pageSize: number;
    /** 适配器类型 */
    storageKind: string;
    /** 数据是否持久化 */
    persistent: boolean;
    /** 事务状态 */
    transactionState: TransactionState;
}

/** 事务内的一处变更 */
interface PendingMutation {
    /** 操作类型 */
    operation: WalOperation.Put | WalOperation.Delete;
    /** 值，删除时为 null */
    value: Uint8Array | null;
}

/**
 * 存储引擎
 */
export class StorageEngine {
    private readonly databaseName: string;
    private readonly adapter: StorageAdapter;
    private readonly store: RecordStore;
    private readonly wal: WalLog;
    private readonly checkpointWriter: CheckpointWriter;
    private readonly broadcaster: DatabaseBroadcaster;
    private readonly lease: WriterLease;
    private readonly options: Required<Omit<StorageEngineOptions, "lease">>;

    private lastLsn: bigint;
    private nextTransactionId: bigint;
    private overlay: Map<string, PendingMutation> | null;
    private closed: boolean;

    private constructor(
        name: string,
        adapter: StorageAdapter,
        wal: WalLog,
        lease: WriterLease,
        options: Required<Omit<StorageEngineOptions, "lease">>,
        lastLsn: bigint,
    ) {
        this.databaseName = name;
        this.adapter = adapter;
        this.wal = wal;
        this.store = new RecordStore();
        this.checkpointWriter = new CheckpointWriter(adapter, options.pageSize);
        this.broadcaster = new DatabaseBroadcaster(name);
        this.lease = lease;
        this.options = options;
        this.lastLsn = lastLsn;
        this.nextTransactionId = 1n;
        this.overlay = null;
        this.closed = false;
    }

    /**
     * 打开数据库
     * @param name 数据库名
     * @param factory 存储适配器工厂
     * @param options 引擎选项
     */
    public static async open(
        name: string,
        factory: StorageAdapterFactory,
        options: StorageEngineOptions = {},
    ): Promise<StorageEngine> {
        const resolved = {
            pageSize: options.pageSize ?? DEFAULT_PAGE_SIZE,
            walThreshold: options.walThreshold ?? DEFAULT_WAL_THRESHOLD,
            autoCheckpoint: options.autoCheckpoint ?? true,
        };

        if (resolved.pageSize < 512) {
            throw new LightDbError(LightDbErrorCodes.unsupported, "page size must be at least 512 bytes", {
                operation: "open",
            });
        }

        const lease = await acquireWriterLease(name, options.lease ?? "web-locks");
        let adapter: StorageAdapter | null = null;

        try {
            adapter = await factory.openDatabase(name);
            const wal = new WalLog(adapter);
            await wal.open();

            const recovery = new RecoveryManager(adapter);
            const outcome = await recovery.recover(wal);

            const engine = new StorageEngine(name, adapter, wal, lease, resolved, outcome.lastLsn);
            engine.store.restore(outcome.records);
            return engine;
        } catch (error) {
            if (adapter) {
                await adapter.close();
            }
            await lease.release();
            if (error instanceof LightDbError) {
                throw error;
            }
            throw new LightDbError(LightDbErrorCodes.io, `failed to open database "${name}"`, {
                operation: "open",
                cause: error,
            });
        }
    }

    /** 数据库名 */
    public get name(): string {
        return this.databaseName;
    }

    /** 当前 LSN */
    public get lsn(): bigint {
        return this.lastLsn;
    }

    /** 事务状态 */
    public get transactionState(): TransactionState {
        return this.overlay ? "active" : "idle";
    }

    /** 是否已关闭 */
    public get isClosed(): boolean {
        return this.closed;
    }

    /**
     * 读取键
     * @param key 键
     */
    public async get(key: string): Promise<Uint8Array | null> {
        this.assertOpen("kvGet");
        this.assertKey(key, "kvGet");

        if (this.overlay?.has(key)) {
            const pending = this.overlay.get(key)!;
            return pending.operation === WalOperation.Delete ? null : BinaryUtils.copy(pending.value!);
        }

        return this.store.get(key);
    }

    /**
     * 写入键
     * @param key 键
     * @param value 值
     */
    public async put(key: string, value: Uint8Array): Promise<void> {
        this.assertOpen("kvPut");
        this.assertKey(key, "kvPut");

        if (this.overlay) {
            this.overlay.set(key, { operation: WalOperation.Put, value: BinaryUtils.copy(value) });
            return;
        }

        await this.applyCommitted([{ operation: WalOperation.Put, key, value: BinaryUtils.copy(value) }]);
    }

    /**
     * 删除键
     * @param key 键
     * @returns 键存在时返回 true
     */
    public async delete(key: string): Promise<boolean> {
        this.assertOpen("kvDelete");
        this.assertKey(key, "kvDelete");

        if (this.overlay) {
            const exists = this.existsInView(key);
            this.overlay.set(key, { operation: WalOperation.Delete, value: null });
            return exists;
        }

        if (!this.store.has(key)) {
            return false;
        }

        await this.applyCommitted([{ operation: WalOperation.Delete, key, value: null }]);
        return true;
    }

    /** 开启事务 */
    public begin(): void {
        this.assertOpen("begin");
        if (this.overlay) {
            throw new LightDbError(LightDbErrorCodes.unsupported, "a transaction is already active", {
                operation: "begin",
            });
        }
        this.overlay = new Map();
    }

    /**
     * 提交事务
     */
    public async commit(): Promise<void> {
        this.assertOpen("commit");
        if (!this.overlay) {
            throw new LightDbError(LightDbErrorCodes.unsupported, "no active transaction", { operation: "commit" });
        }

        const pending = this.overlay;
        this.overlay = null;

        if (pending.size === 0) {
            return;
        }

        const transactionId = this.nextTransactionId++;
        const mutations: { operation: WalOperation.Put | WalOperation.Delete; key: string; value: Uint8Array | null }[] = [];

        for (const [key, mutation] of pending) {
            mutations.push({ operation: mutation.operation, key, value: mutation.value });
        }

        await this.applyCommitted(mutations, transactionId);
    }

    /** 回滚事务 */
    public rollback(): void {
        this.assertOpen("rollback");
        if (!this.overlay) {
            throw new LightDbError(LightDbErrorCodes.unsupported, "no active transaction", { operation: "rollback" });
        }
        this.overlay = null;
    }

    /**
     * 执行检查点
     */
    public async checkpoint(): Promise<CheckpointResult> {
        this.assertOpen("checkpoint");
        if (this.overlay) {
            throw new LightDbError(LightDbErrorCodes.unsupported, "cannot checkpoint during a transaction", {
                operation: "checkpoint",
            });
        }

        const walBytesReleased = this.wal.byteSize;
        const bytesWritten = await this.checkpointWriter.write(this.store.snapshot(), this.lastLsn);
        await this.wal.truncate(0);

        return { lsn: this.lastLsn, bytesWritten, walBytesReleased };
    }

    /** 广播 schema 变更，供其它标签页失效本地视图 */
    public notifySchemaChange(): void {
        this.assertOpen("notifySchemaChange");
        this.broadcaster.notify({ type: "schema", name: this.databaseName });
    }

    /** 统计信息 */
    public getStats(): StorageEngineStats {
        this.assertOpen("stats");
        return {
            name: this.databaseName,
            lsn: this.lastLsn,
            recordCount: this.store.size,
            walBytes: this.wal.byteSize,
            pageSize: this.options.pageSize,
            storageKind: this.adapter.kind,
            persistent: this.adapter.persistent,
            transactionState: this.transactionState,
        };
    }

    /** 关闭引擎 */
    public async close(): Promise<void> {
        if (this.closed) {
            return;
        }

        this.closed = true;
        this.overlay = null;

        this.broadcaster.notify({ type: "close", name: this.databaseName });
        this.broadcaster.close();

        try {
            await this.wal.close();
        } finally {
            try {
                await this.adapter.close();
            } finally {
                await this.lease.release();
            }
        }
    }

    /**
     * 追加 WAL 并应用记录
     * @param mutations 变更
     * @param transactionId 事务号
     */
    private async applyCommitted(
        mutations: readonly { operation: WalOperation.Put | WalOperation.Delete; key: string; value: Uint8Array | null }[],
        transactionId: bigint = 0n,
    ): Promise<void> {
        const records = mutations.map((mutation) => ({
            lsn: ++this.lastLsn,
            transactionId,
            operation: mutation.operation,
            key: mutation.key,
            value: mutation.value,
        }));

        await this.wal.append(records);

        for (const mutation of mutations) {
            if (mutation.operation === WalOperation.Delete) {
                this.store.delete(mutation.key);
            } else if (mutation.value) {
                this.store.put(mutation.key, mutation.value);
            }
        }

        this.broadcaster.notify({ type: "commit", name: this.databaseName, lsn: this.lastLsn.toString() });

        if (this.options.autoCheckpoint && this.wal.byteSize >= this.options.walThreshold) {
            await this.checkpoint();
        }
    }

    /**
     * 判断键在当前视图（含事务覆盖层）是否存在
     * @param key 键
     */
    private existsInView(key: string): boolean {
        if (this.overlay?.has(key)) {
            return this.overlay.get(key)!.operation === WalOperation.Put;
        }
        return this.store.has(key);
    }

    /**
     * 断言引擎未关闭
     * @param operation 操作名
     */
    private assertOpen(operation: string): void {
        if (this.closed) {
            throw new LightDbError(LightDbErrorCodes.sessionClosed, `database "${this.databaseName}" is closed`, {
                operation,
            });
        }
    }

    /**
     * 断言键合法
     * @param key 键
     * @param operation 操作名
     */
    private assertKey(key: string, operation: string): void {
        if (key.length === 0) {
            throw new LightDbError(LightDbErrorCodes.unsupported, "key must not be empty", { operation });
        }
    }
}
