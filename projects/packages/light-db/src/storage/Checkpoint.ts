/**
 * 检查点（Checkpoint）机制实现
 * 将内存中的完整数据原子性写入新的 .light 文件
 */

import { DEFAULT_WAL_THRESHOLD } from "../constants";
import { CheckpointResult, RecordMap, PageMap } from "../types";
import { LightFileManager, PageManager } from "./LightFile";
import { WalManager } from "./WALFile";
import { WalIndexManager } from "./SHMFile";
import { CheckpointError } from "../errors";

/**
 * 检查点配置选项
 */
export interface CheckpointOptions {
    /** 是否强制执行检查点（忽略阈值检查） */
    force?: boolean;
    /** 是否同步等待检查点完成 */
    sync?: boolean;
}

/**
 * 检查点管理器
 * 负责管理检查点的创建和协调
 */
export class CheckpointManager {
    private readonly lightFileManager: LightFileManager;
    private readonly walManager: WalManager;
    private readonly walIndexManager: WalIndexManager;
    private readonly pageManager: PageManager;
    private readonly walThreshold: number;
    private lastCheckpointTime: number | null;
    private checkpointInProgress: boolean;

    /**
     * 创建检查点管理器
     * @param lightFileManager Light 文件管理器
     * @param walManager WAL 管理器
     * @param walIndexManager WAL 索引管理器
     * @param pageManager 页面管理器
     * @param walThreshold WAL 文件大小阈值
     */
    constructor(
        lightFileManager: LightFileManager,
        walManager: WalManager,
        walIndexManager: WalIndexManager,
        pageManager: PageManager,
        walThreshold: number = DEFAULT_WAL_THRESHOLD,
    ) {
        this.lightFileManager = lightFileManager;
        this.walManager = walManager;
        this.walIndexManager = walIndexManager;
        this.pageManager = pageManager;
        this.walThreshold = walThreshold;
        this.lastCheckpointTime = null;
        this.checkpointInProgress = false;
    }

    /**
     * 检查是否需要执行检查点
     */
    public async shouldCheckpoint(): Promise<boolean> {
        if (this.checkpointInProgress) {
            return false;
        }

        const walSize = await this.walManager.getFileSize();
        return walSize >= this.walThreshold;
    }

    /**
     * 执行检查点
     * @param records 当前内存中的所有记录
     */
    public async execute(records: RecordMap): Promise<CheckpointResult> {
        if (this.checkpointInProgress) {
            throw new CheckpointError("Checkpoint already in progress");
        }

        this.checkpointInProgress = true;

        try {
            const currentLsn = this.walManager.getCurrentLsn();

            await this.walManager.writeCheckpoint();
            await this.walManager.sync();

            const pages = this.buildPages(records);

            await this.lightFileManager.writeSnapshot(pages, currentLsn);

            await this.walManager.clear();

            await this.walIndexManager.resetAfterCheckpoint(currentLsn);

            this.pageManager.loadPages(pages);
            this.pageManager.clearDirtyFlags();

            this.lastCheckpointTime = Date.now();

            const endTime = Date.now();
            const bytesWritten = await this.lightFileManager.getFileSize();

            return {
                timestamp: endTime,
                lsn: currentLsn,
                pagesWritten: pages.size,
                bytesWritten,
            };
        } catch (error) {
            const err = error as Error;
            throw new CheckpointError(`Checkpoint failed: ${err.message}`, this.walManager.getCurrentLsn());
        } finally {
            this.checkpointInProgress = false;
        }
    }

    /**
     * 将记录转换为页面
     * @param records 记录映射
     */
    private buildPages(records: RecordMap): PageMap {
        const pages: PageMap = new Map();
        const pageSize = this.pageManager.getPageSize();
        let currentPageId = 0;
        let currentRecords: Array<{ key: string; value: Buffer }> = [];
        let currentSize = 0;

        const headerSize = 12;
        const recordOverhead = 4 + 4 + 4;

        for (const [key, value] of records) {
            const keyLength = Buffer.byteLength(key, "utf8");
            const recordSize = recordOverhead + keyLength + value.length;

            if (currentSize + recordSize > pageSize - headerSize - 4 && currentRecords.length > 0) {
                pages.set(currentPageId, {
                    pageId: currentPageId,
                    records: currentRecords.map((r) => ({ key: r.key, value: r.value })),
                    checksum: 0,
                });

                currentPageId++;
                currentRecords = [];
                currentSize = 0;
            }

            currentRecords.push({ key, value });
            currentSize += recordSize;
        }

        if (currentRecords.length > 0) {
            pages.set(currentPageId, {
                pageId: currentPageId,
                records: currentRecords.map((r) => ({ key: r.key, value: r.value })),
                checksum: 0,
            });
        }

        return pages;
    }

    /**
     * 获取上次检查点时间
     */
    public getLastCheckpointTime(): number | null {
        return this.lastCheckpointTime;
    }

    /**
     * 检查是否有检查点正在进行
     */
    public isCheckpointInProgress(): boolean {
        return this.checkpointInProgress;
    }

    /**
     * 获取 WAL 阈值
     */
    public getWalThreshold(): number {
        return this.walThreshold;
    }
}

/**
 * 自动检查点调度器
 * 根据配置自动触发检查点
 */
export class AutoCheckpointScheduler {
    private readonly checkpointManager: CheckpointManager;
    private readonly interval: number;
    private timer: NodeJS.Timeout | null;
    private running: boolean;
    private getRecords: () => RecordMap;

    /**
     * 创建自动检查点调度器
     * @param checkpointManager 检查点管理器
     * @param interval 检查间隔（毫秒）
     * @param getRecords 获取当前记录的函数
     */
    constructor(checkpointManager: CheckpointManager, interval: number, getRecords: () => RecordMap) {
        this.checkpointManager = checkpointManager;
        this.interval = interval;
        this.timer = null;
        this.running = false;
        this.getRecords = getRecords;
    }

    /**
     * 启动调度器
     */
    public start(): void {
        if (this.running) {
            return;
        }

        this.running = true;
        this.timer = setInterval(async () => {
            try {
                if (await this.checkpointManager.shouldCheckpoint()) {
                    await this.checkpointManager.execute(this.getRecords());
                }
            } catch {
                // 静默处理错误，避免影响定时器
            }
        }, this.interval);
    }

    /**
     * 停止调度器
     */
    public stop(): void {
        if (this.timer) {
            clearInterval(this.timer);
            this.timer = null;
        }
        this.running = false;
    }

    /**
     * 检查是否正在运行
     */
    public isRunning(): boolean {
        return this.running;
    }
}

/**
 * 增量检查点管理器
 * 只将脏页写入文件，减少 I/O
 */
export class IncrementalCheckpointManager {
    private readonly lightFileManager: LightFileManager;
    private readonly walManager: WalManager;
    private readonly pageManager: PageManager;
    private readonly walThreshold: number;

    /**
     * 创建增量检查点管理器
     * @param lightFileManager Light 文件管理器
     * @param walManager WAL 管理器
     * @param pageManager 页面管理器
     * @param walThreshold WAL 阈值
     */
    constructor(
        lightFileManager: LightFileManager,
        walManager: WalManager,
        pageManager: PageManager,
        walThreshold: number = DEFAULT_WAL_THRESHOLD,
    ) {
        this.lightFileManager = lightFileManager;
        this.walManager = walManager;
        this.pageManager = pageManager;
        this.walThreshold = walThreshold;
    }

    /**
     * 执行增量检查点
     */
    public async executeIncremental(): Promise<CheckpointResult> {
        const dirtyPages = this.pageManager.getDirtyPages();

        if (dirtyPages.length === 0) {
            return {
                timestamp: Date.now(),
                lsn: this.walManager.getCurrentLsn(),
                pagesWritten: 0,
                bytesWritten: 0,
            };
        }

        const currentLsn = this.walManager.getCurrentLsn();

        for (const page of dirtyPages) {
            await this.lightFileManager.writePage(page);
        }

        await this.lightFileManager.sync();

        const header = await this.lightFileManager.readHeader();
        if (header) {
            header.lastSnapshotLsn = currentLsn;
            await this.lightFileManager.writeHeader(header);
        }

        this.pageManager.clearDirtyFlags();

        const totalBytes = dirtyPages.length * this.pageManager.getPageSize();

        return {
            timestamp: Date.now(),
            lsn: currentLsn,
            pagesWritten: dirtyPages.length,
            bytesWritten: totalBytes,
        };
    }

    /**
     * 检查是否需要增量检查点
     */
    public async shouldCheckpoint(): Promise<boolean> {
        const dirtyPages = this.pageManager.getDirtyPages();
        if (dirtyPages.length > 100) {
            return true;
        }

        const walSize = await this.walManager.getFileSize();
        return walSize >= this.walThreshold;
    }
}
