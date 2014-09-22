/**
 * 存储引擎核心模块
 * 整合所有存储组件，提供统一的 API
 */

import { DEFAULT_PAGE_SIZE, DEFAULT_WAL_THRESHOLD } from "../constants";
import { StorageEngineOptions, WriteResult, BatchWriteResult, CheckpointResult, RecoveryResult, StorageStats, RecordMap } from "../types";
import { FileUtils } from "../utils";
import { LightFileManager, PageManager } from "./LightFile";
import { WalManager } from "./WALFile";
import { WalIndexManager } from "./SHMFile";
import { CheckpointManager, AutoCheckpointScheduler } from "./Checkpoint";
import { RecoveryManager } from "./Recovery";
import { StorageError } from "../errors";

/**
 * 存储引擎
 * LightDB 的核心存储层，提供完整的数据持久化能力
 */
export class StorageEngine {
    private readonly basePath: string;
    private readonly dbName: string;
    private readonly options: Required<StorageEngineOptions>;

    private lightFileManager: LightFileManager | null = null;
    private walManager: WalManager | null = null;
    private walIndexManager: WalIndexManager | null = null;
    private pageManager: PageManager | null = null;
    private checkpointManager: CheckpointManager | null = null;
    private autoCheckpointScheduler: AutoCheckpointScheduler | null = null;

    private records: RecordMap;
    private initialized: boolean;
    private closed: boolean;

    /**
     * 创建存储引擎实例
     * @param basePath 数据库基础路径
     * @param dbName 数据库名称
     * @param options 配置选项
     */
    constructor(basePath: string, dbName: string, options: StorageEngineOptions = {}) {
        this.basePath = basePath;
        this.dbName = dbName;
        this.options = {
            pageSize: options.pageSize ?? DEFAULT_PAGE_SIZE,
            walThreshold: options.walThreshold ?? DEFAULT_WAL_THRESHOLD,
            autoCheckpoint: options.autoCheckpoint ?? true,
            fsyncStrategy: options.fsyncStrategy ?? { type: "always" },
        };

        this.records = new Map();
        this.initialized = false;
        this.closed = false;
    }

    /**
     * 初始化存储引擎
     * 创建必要的目录和文件，执行恢复流程
     */
    public async initialize(): Promise<RecoveryResult> {
        if (this.initialized) {
            return {
                success: true,
                recordsReplayed: 0,
                pagesLoaded: 0,
                durationMs: 0,
            };
        }

        await FileUtils.ensureDir(this.basePath);

        this.lightFileManager = new LightFileManager(this.basePath, this.dbName, this.options.pageSize);

        this.walManager = new WalManager(this.basePath, this.dbName);
        await this.walManager.initialize();

        this.walIndexManager = new WalIndexManager(this.basePath, this.dbName);
        await this.walIndexManager.initialize();

        this.pageManager = new PageManager(this.options.pageSize);

        this.checkpointManager = new CheckpointManager(
            this.lightFileManager,
            this.walManager,
            this.walIndexManager,
            this.pageManager,
            this.options.walThreshold,
        );

        const recoveryManager = new RecoveryManager(
            this.basePath,
            this.dbName,
            this.lightFileManager,
            this.walManager,
            this.walIndexManager,
            this.pageManager,
        );

        const recoveryResult = await recoveryManager.recover();

        if (recoveryResult.success) {
            const recoveredRecords = await recoveryManager.getRecoveredRecords();
            this.records = recoveredRecords;
            this.initialized = true;

            if (this.options.autoCheckpoint) {
                this.startAutoCheckpoint();
            }
        }

        return recoveryResult;
    }

    /**
     * 启动自动检查点
     */
    private startAutoCheckpoint(): void {
        if (this.autoCheckpointScheduler) {
            return;
        }

        this.autoCheckpointScheduler = new AutoCheckpointScheduler(this.checkpointManager!, 5000, () => this.records);
        this.autoCheckpointScheduler.start();
    }

    /**
     * 停止自动检查点
     */
    private stopAutoCheckpoint(): void {
        if (this.autoCheckpointScheduler) {
            this.autoCheckpointScheduler.stop();
            this.autoCheckpointScheduler = null;
        }
    }

    /**
     * 确保引擎已初始化
     */
    private ensureInitialized(): void {
        if (!this.initialized) {
            throw new StorageError("Storage engine not initialized", "NOT_INITIALIZED");
        }
        if (this.closed) {
            throw new StorageError("Storage engine is closed", "ENGINE_CLOSED");
        }
    }

    /**
     * 插入记录
     * @param key 键
     * @param value 值
     */
    public async insert(key: string, value: Buffer): Promise<WriteResult> {
        this.ensureInitialized();

        if (this.records.has(key)) {
            throw new StorageError(`Key already exists: ${key}`, "KEY_EXISTS");
        }

        const result = await this.walManager!.writeInsert(key, value);

        if (result.success) {
            this.records.set(key, value);
        }

        return result;
    }

    /**
     * 更新记录
     * @param key 键
     * @param value 新值
     */
    public async update(key: string, value: Buffer): Promise<WriteResult> {
        this.ensureInitialized();

        if (!this.records.has(key)) {
            throw new StorageError(`Key not found: ${key}`, "KEY_NOT_FOUND");
        }

        const result = await this.walManager!.writeUpdate(key, value);

        if (result.success) {
            this.records.set(key, value);
        }

        return result;
    }

    /**
     * 插入或更新记录
     * @param key 键
     * @param value 值
     */
    public async upsert(key: string, value: Buffer): Promise<WriteResult> {
        this.ensureInitialized();

        const exists = this.records.has(key);
        const result = exists ? await this.walManager!.writeUpdate(key, value) : await this.walManager!.writeInsert(key, value);

        if (result.success) {
            this.records.set(key, value);
        }

        return result;
    }

    /**
     * 删除记录
     * @param key 键
     */
    public async delete(key: string): Promise<WriteResult> {
        this.ensureInitialized();

        if (!this.records.has(key)) {
            throw new StorageError(`Key not found: ${key}`, "KEY_NOT_FOUND");
        }

        const result = await this.walManager!.writeDelete(key);

        if (result.success) {
            this.records.delete(key);
        }

        return result;
    }

    /**
     * 读取记录
     * @param key 键
     */
    public async get(key: string): Promise<Buffer | null> {
        this.ensureInitialized();
        return this.records.get(key) ?? null;
    }

    /**
     * 检查记录是否存在
     * @param key 键
     */
    public async exists(key: string): Promise<boolean> {
        this.ensureInitialized();
        return this.records.has(key);
    }

    /**
     * 批量插入
     * @param entries 键值对数组
     */
    public async insertBatch(entries: Array<{ key: string; value: Buffer }>): Promise<BatchWriteResult> {
        this.ensureInitialized();

        for (const { key } of entries) {
            if (this.records.has(key)) {
                throw new StorageError(`Key already exists: ${key}`, "KEY_EXISTS");
            }
        }

        const results: WriteResult[] = [];
        let startLsn = 0n;
        let endLsn = 0n;
        let totalBytes = 0;

        for (let i = 0; i < entries.length; i++) {
            const { key, value } = entries[i]!;
            const result = await this.walManager!.writeInsert(key, value);
            results.push(result);

            if (i === 0) {
                startLsn = result.lsn;
            }
            endLsn = result.lsn;
            totalBytes += result.bytesWritten;

            if (result.success) {
                this.records.set(key, value);
            }
        }

        return {
            success: results.every((r) => r.success),
            startLsn,
            endLsn,
            totalBytesWritten: totalBytes,
            recordsWritten: results.length,
        };
    }

    /**
     * 执行检查点
     */
    public async checkpoint(): Promise<CheckpointResult> {
        this.ensureInitialized();
        return this.checkpointManager!.execute(this.records);
    }

    /**
     * 获取所有键
     */
    public async getAllKeys(): Promise<string[]> {
        this.ensureInitialized();
        return Array.from(this.records.keys());
    }

    /**
     * 获取所有记录
     */
    public async getAllRecords(): Promise<RecordMap> {
        this.ensureInitialized();
        return new Map(this.records);
    }

    /**
     * 获取记录数量
     */
    public async getRecordCount(): Promise<number> {
        this.ensureInitialized();
        return this.records.size;
    }

    /**
     * 获取存储统计信息
     */
    public async getStats(): Promise<StorageStats> {
        this.ensureInitialized();

        return {
            currentLsn: this.walManager!.getCurrentLsn(),
            currentTransactionId: this.walManager!.getCurrentTransactionId(),
            walFileSize: await this.walManager!.getFileSize(),
            lightFileSize: await this.lightFileManager!.getFileSize(),
            pageCount: this.pageManager!.getPageCount(),
            recordCount: this.records.size,
            lastCheckpointTime: this.checkpointManager!.getLastCheckpointTime(),
        };
    }

    /**
     * 同步数据到磁盘
     */
    public async sync(): Promise<void> {
        this.ensureInitialized();
        await this.walManager!.sync();
        await this.walIndexManager!.sync();
    }

    /**
     * 关闭存储引擎
     */
    public async close(): Promise<void> {
        if (this.closed) {
            return;
        }

        this.stopAutoCheckpoint();

        if (this.initialized) {
            await this.sync();

            if (this.walManager) {
                await this.walManager.close();
            }

            if (this.walIndexManager) {
                await this.walIndexManager.close();
            }

            if (this.lightFileManager) {
                await this.lightFileManager.close();
            }
        }

        this.closed = true;
        this.initialized = false;
    }

    /**
     * 检查是否已初始化
     */
    public isInitialized(): boolean {
        return this.initialized;
    }

    /**
     * 检查是否已关闭
     */
    public isClosed(): boolean {
        return this.closed;
    }

    /**
     * 获取基础路径
     */
    public getBasePath(): string {
        return this.basePath;
    }

    /**
     * 获取数据库名称
     */
    public getDbName(): string {
        return this.dbName;
    }

    /**
     * 获取页面大小
     */
    public getPageSize(): number {
        return this.options.pageSize;
    }
}

/**
 * 创建存储引擎实例的工厂函数
 * @param basePath 数据库基础路径
 * @param dbName 数据库名称
 * @param options 配置选项
 */
export async function createStorageEngine(basePath: string, dbName: string, options: StorageEngineOptions = {}): Promise<StorageEngine> {
    const engine = new StorageEngine(basePath, dbName, options);
    await engine.initialize();
    return engine;
}
