/**
 * 崩溃恢复流程实现
 * 负责数据库启动时的恢复操作
 */

import { OperationType } from "../constants";
import { RecoveryResult, RecordMap, WalRecord } from "../types";
import { LightFileManager, PageManager } from "./LightFile";
import { WalManager, WalReader } from "./WALFile";
import { WalIndexManager } from "./SHMFile";
import { RecoveryError } from "../errors";

/**
 * 恢复阶段枚举
 */
export enum RecoveryPhase {
    /** 初始化阶段 */
    Initialize = "initialize",
    /** 加载 SHM 索引 */
    LoadSHM = "load_shm",
    /** 加载 Light 文件 */
    LoadLight = "load_light",
    /** 回放 WAL */
    ReplayWAL = "replay_wal",
    /** 重建索引 */
    RebuildIndex = "rebuild_index",
    /** 完成 */
    Complete = "complete",
}

/**
 * 恢复上下文
 * 存储恢复过程中的状态信息
 */
export interface RecoveryContext {
    /** 当前阶段 */
    phase: RecoveryPhase;
    /** SHM 是否有效 */
    shmValid: boolean;
    /** Light 文件是否存在 */
    lightExists: boolean;
    /** WAL 文件是否存在 */
    walExists: boolean;
    /** 最后检查点 LSN */
    lastCheckpointLsn: bigint;
    /** 需要回放的 WAL 记录数 */
    recordsToReplay: number;
    /** 已回放的记录数 */
    recordsReplayed: number;
    /** 错误信息 */
    error?: string;
}

/**
 * 恢复管理器
 * 负责数据库启动时的完整恢复流程
 */
export class RecoveryManager {
    private readonly basePath: string;
    private readonly dbName: string;
    private readonly lightFileManager: LightFileManager;
    private readonly walManager: WalManager;
    private readonly walIndexManager: WalIndexManager;
    private readonly pageManager: PageManager;
    private context: RecoveryContext;

    /**
     * 创建恢复管理器
     * @param basePath 数据库基础路径
     * @param dbName 数据库名称
     * @param lightFileManager Light 文件管理器
     * @param walManager WAL 管理器
     * @param walIndexManager WAL 索引管理器
     * @param pageManager 页面管理器
     */
    constructor(
        basePath: string,
        dbName: string,
        lightFileManager: LightFileManager,
        walManager: WalManager,
        walIndexManager: WalIndexManager,
        pageManager: PageManager,
    ) {
        this.basePath = basePath;
        this.dbName = dbName;
        this.lightFileManager = lightFileManager;
        this.walManager = walManager;
        this.walIndexManager = walIndexManager;
        this.pageManager = pageManager;
        this.context = {
            phase: RecoveryPhase.Initialize,
            shmValid: false,
            lightExists: false,
            walExists: false,
            lastCheckpointLsn: 0n,
            recordsToReplay: 0,
            recordsReplayed: 0,
        };
    }

    /**
     * 执行完整恢复流程
     */
    public async recover(): Promise<RecoveryResult> {
        const startTime = Date.now();
        const records: RecordMap = new Map();

        try {
            this.context.phase = RecoveryPhase.Initialize;
            await this.initializeRecovery();

            this.context.phase = RecoveryPhase.LoadSHM;
            await this.loadShmIndex();

            this.context.phase = RecoveryPhase.LoadLight;
            const lightRecords = await this.loadLightFile();
            for (const [key, value] of lightRecords) {
                records.set(key, value);
            }

            this.context.phase = RecoveryPhase.ReplayWAL;
            const walRecords = await this.replayWal(records);
            this.context.recordsReplayed = walRecords;

            this.context.phase = RecoveryPhase.RebuildIndex;
            await this.rebuildIndex();

            this.context.phase = RecoveryPhase.Complete;

            const endTime = Date.now();
            const durationMs = endTime - startTime;

            return {
                success: true,
                recordsReplayed: walRecords,
                pagesLoaded: this.pageManager.getPageCount(),
                durationMs,
            };
        } catch (error) {
            const err = error as Error;
            this.context.error = err.message;

            return {
                success: false,
                recordsReplayed: this.context.recordsReplayed,
                pagesLoaded: this.pageManager.getPageCount(),
                durationMs: Date.now() - startTime,
                error: err.message,
            };
        }
    }

    /**
     * 初始化恢复
     */
    private async initializeRecovery(): Promise<void> {
        this.context.lightExists = await this.lightFileManager.exists();
        this.context.walExists = await this.checkWalExists();

        if (this.context.lightExists) {
            const header = await this.lightFileManager.readHeader();
            if (header) {
                this.context.lastCheckpointLsn = header.lastSnapshotLsn;
            }
        }
    }

    /**
     * 检查 WAL 文件是否存在
     */
    private async checkWalExists(): Promise<boolean> {
        const reader = new WalReader(this.basePath, this.dbName);
        await reader.open();
        const size = reader.getFileSize();
        await reader.close();
        return size > 0;
    }

    /**
     * 加载 SHM 索引
     */
    private async loadShmIndex(): Promise<void> {
        const shmManager = this.walIndexManager.getShmManager();
        this.context.shmValid = await shmManager.load();

        if (!this.context.shmValid) {
            // SHM 无效，需要重建
        }
    }

    /**
     * 加载 Light 文件
     */
    private async loadLightFile(): Promise<RecordMap> {
        if (!this.context.lightExists) {
            return new Map();
        }

        await this.lightFileManager.open();
        const records = await this.lightFileManager.loadAllRecords();
        await this.lightFileManager.close();

        return records;
    }

    /**
     * 回放 WAL 记录
     * @param records 当前记录映射
     */
    private async replayWal(records: RecordMap): Promise<number> {
        const reader = new WalReader(this.basePath, this.dbName);
        await reader.open();

        let replayCount = 0;
        let lastLsn = 0n;

        try {
            while (true) {
                const record = await reader.readNext();
                if (!record) {
                    break;
                }

                if (record.lsn <= this.context.lastCheckpointLsn) {
                    continue;
                }

                this.applyWalRecord(records, record);
                replayCount++;
                lastLsn = record.lsn;
            }

            this.walManager.setCurrentLsn(lastLsn);
        } finally {
            await reader.close();
        }

        this.context.recordsToReplay = replayCount;
        return replayCount;
    }

    /**
     * 应用单条 WAL 记录
     * @param records 记录映射
     * @param record WAL 记录
     */
    private applyWalRecord(records: RecordMap, record: WalRecord): void {
        switch (record.operationType) {
            case OperationType.Insert:
            case OperationType.Update:
                if (record.value) {
                    records.set(record.key, record.value);
                }
                break;

            case OperationType.Delete:
                records.delete(record.key);
                break;

            case OperationType.Checkpoint:
                // 检查点标记，不需要特殊处理
                break;

            default:
                throw new RecoveryError(`Unknown operation type: ${record.operationType}`, "replay_wal", { lsn: record.lsn });
        }
    }

    /**
     * 重建索引
     */
    private async rebuildIndex(): Promise<void> {
        // 重建 SHM 索引
        const shmManager = this.walIndexManager.getShmManager();
        await shmManager.clear();
    }

    /**
     * 获取恢复上下文
     */
    public getContext(): RecoveryContext {
        return { ...this.context };
    }

    /**
     * 获取恢复后的记录
     */
    public async getRecoveredRecords(): Promise<RecordMap> {
        const records: RecordMap = new Map();

        // 从 Light 文件加载
        if (this.context.lightExists) {
            await this.lightFileManager.open();
            const lightRecords = await this.lightFileManager.loadAllRecords();
            for (const [key, value] of lightRecords) {
                records.set(key, value);
            }
            await this.lightFileManager.close();
        }

        // 回放 WAL
        const reader = new WalReader(this.basePath, this.dbName);
        await reader.open();

        try {
            while (true) {
                const record = await reader.readNext();
                if (!record) {
                    break;
                }

                if (record.lsn > this.context.lastCheckpointLsn) {
                    this.applyWalRecord(records, record);
                }
            }
        } finally {
            await reader.close();
        }

        return records;
    }
}

/**
 * 快速恢复管理器
 * 利用 SHM 索引加速恢复过程
 */
export class FastRecoveryManager {
    private readonly basePath: string;
    private readonly dbName: string;
    private readonly lightFileManager: LightFileManager;
    private readonly walManager: WalManager;
    private readonly walIndexManager: WalIndexManager;

    /**
     * 创建快速恢复管理器
     * @param basePath 数据库基础路径
     * @param dbName 数据库名称
     * @param lightFileManager Light 文件管理器
     * @param walManager WAL 管理器
     * @param walIndexManager WAL 索引管理器
     */
    constructor(
        basePath: string,
        dbName: string,
        lightFileManager: LightFileManager,
        walManager: WalManager,
        walIndexManager: WalIndexManager,
    ) {
        this.basePath = basePath;
        this.dbName = dbName;
        this.lightFileManager = lightFileManager;
        this.walManager = walManager;
        this.walIndexManager = walIndexManager;
    }

    /**
     * 快速恢复
     * 只回放必要的 WAL 记录
     */
    public async fastRecover(): Promise<RecoveryResult> {
        const startTime = Date.now();
        const records: RecordMap = new Map();

        try {
            // 1. 加载 Light 文件
            const lightExists = await this.lightFileManager.exists();
            if (lightExists) {
                await this.lightFileManager.open();
                const lightRecords = await this.lightFileManager.loadAllRecords();
                for (const [key, value] of lightRecords) {
                    records.set(key, value);
                }
            }

            // 2. 加载 SHM 索引
            const shmManager = this.walIndexManager.getShmManager();
            const shmValid = await shmManager.load();

            // 3. 获取需要回放的 WAL 范围
            let startLsn = 0n;
            if (shmValid) {
                startLsn = shmManager.getLastCompleteLsn();
            }

            // 4. 回放 WAL
            const reader = new WalReader(this.basePath, this.dbName);
            await reader.open();

            let replayCount = 0;
            let lastLsn = 0n;

            try {
                while (true) {
                    const record = await reader.readNext();
                    if (!record) {
                        break;
                    }

                    if (record.lsn > startLsn) {
                        this.applyRecord(records, record);
                        replayCount++;
                    }
                    lastLsn = record.lsn;
                }
            } finally {
                await reader.close();
            }

            // 5. 更新 LSN
            if (lastLsn > 0n) {
                this.walManager.setCurrentLsn(lastLsn);
            }

            const endTime = Date.now();

            return {
                success: true,
                recordsReplayed: replayCount,
                pagesLoaded: lightExists ? 1 : 0,
                durationMs: endTime - startTime,
            };
        } catch (error) {
            const err = error as Error;
            return {
                success: false,
                recordsReplayed: 0,
                pagesLoaded: 0,
                durationMs: Date.now() - startTime,
                error: err.message,
            };
        }
    }

    /**
     * 应用 WAL 记录
     */
    private applyRecord(records: RecordMap, record: WalRecord): void {
        switch (record.operationType) {
            case OperationType.Insert:
            case OperationType.Update:
                if (record.value) {
                    records.set(record.key, record.value);
                }
                break;
            case OperationType.Delete:
                records.delete(record.key);
                break;
        }
    }
}

/**
 * 恢复验证器
 * 验证恢复后的数据完整性
 */
export class RecoveryValidator {
    /**
     * 验证记录完整性
     * @param records 记录映射
     */
    public validateRecords(records: RecordMap): {
        valid: boolean;
        errors: string[];
    } {
        const errors: string[] = [];

        for (const [key, value] of records) {
            if (!key || key.length === 0) {
                errors.push(`Invalid key: empty key found`);
            }

            if (!value || value.length === 0) {
                errors.push(`Invalid value for key "${key}": empty value`);
            }
        }

        return {
            valid: errors.length === 0,
            errors,
        };
    }

    /**
     * 验证 WAL 完整性
     * @param walRecords WAL 记录数组
     */
    public validateWAL(walRecords: WalRecord[]): {
        valid: boolean;
        errors: string[];
    } {
        const errors: string[] = [];
        let lastLsn = 0n;

        for (const record of walRecords) {
            if (record.lsn <= lastLsn) {
                errors.push(`LSN not monotonic: ${lastLsn} -> ${record.lsn}`);
            }
            lastLsn = record.lsn;

            if (!Object.values(OperationType).includes(record.operationType)) {
                errors.push(`Invalid operation type: ${record.operationType}`);
            }
        }

        return {
            valid: errors.length === 0,
            errors,
        };
    }
}
