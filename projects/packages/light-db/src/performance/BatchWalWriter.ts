/**
 * 异步批量写入器
 * 将多个写操作的 WAL 记录暂存于内存队列，定时或队列满时一次性写入
 */

import * as fs from "fs/promises";
import type { FileHandle } from "fs/promises";
import { BatchWriteOptions, BatchWriteStats, BatchFlushResult, FsyncStrategyType } from "./types";
import { OptimizedBufferWriter, BufferConcatenator } from "./OptimizedBuffer";

/**
 * WAL 记录接口
 */
export interface WalRecordEntry {
    /** 日志序列号 */
    lsn: bigint;
    /** 事务 ID */
    transactionId: bigint;
    /** 操作类型 */
    operationType: number;
    /** 键 */
    key: string;
    /** 值 */
    value: Buffer | null;
}

/**
 * 批量写入队列项
 */
interface QueueEntry {
    record: WalRecordEntry;
    buffer: Buffer;
}

/**
 * 异步批量 WAL 写入器
 * 支持合并写入、定时刷新和灵活的 fsync 策略
 */
export class BatchWalWriter {
    private filePath: string;
    private handle: FileHandle | null = null;
    private queue: QueueEntry[];
    private queueCapacity: number;
    private batchSize: number;
    private flushIntervalMs: number;
    private fsyncStrategy: FsyncStrategyType;
    private fsyncBatchSize: number;
    private fsyncIntervalMs: number;

    private flushTimer: NodeJS.Timeout | null = null;
    private fsyncTimer: NodeJS.Timeout | null = null;
    private currentOffset: bigint;
    private writeCountSinceFsync: number;
    private lastFlushTime: number;
    private lastFsyncTime: number;
    private totalWritten: number;
    private isFlushing: boolean;
    private pendingFlush: boolean;
    private closed: boolean;

    /**
     * 创建批量 WAL 写入器
     * @param filePath 文件路径
     * @param options 配置选项
     */
    constructor(filePath: string, options: BatchWriteOptions = {}) {
        this.filePath = filePath;
        this.queue = [];
        this.queueCapacity = options.batchSize ? options.batchSize * 2 : 2000;
        this.batchSize = options.batchSize ?? 100;
        this.flushIntervalMs = options.flushIntervalMs ?? 100;
        this.fsyncStrategy = options.fsyncStrategy ?? "batch";
        this.fsyncBatchSize = options.fsyncBatchSize ?? 1000;
        this.fsyncIntervalMs = options.fsyncIntervalMs ?? 1000;

        this.currentOffset = 0n;
        this.writeCountSinceFsync = 0;
        this.lastFlushTime = 0;
        this.lastFsyncTime = 0;
        this.totalWritten = 0;
        this.isFlushing = false;
        this.pendingFlush = false;
        this.closed = false;
    }

    /**
     * 打开文件
     */
    public async open(): Promise<void> {
        if (this.handle) {
            return;
        }

        try {
            const stats = await fs.stat(this.filePath);
            this.handle = await fs.open(this.filePath, "r+");
            this.currentOffset = BigInt(stats.size);
        } catch {
            this.handle = await fs.open(this.filePath, "w+");
            this.currentOffset = 0n;
        }

        this.startTimers();
    }

    /**
     * 启动定时器
     */
    private startTimers(): void {
        if (this.flushIntervalMs > 0) {
            this.flushTimer = setInterval(() => {
                this.flush().catch(() => {});
            }, this.flushIntervalMs);
        }

        if (this.fsyncStrategy === "interval" && this.fsyncIntervalMs > 0) {
            this.fsyncTimer = setInterval(() => {
                this.fsync().catch(() => {});
            }, this.fsyncIntervalMs);
        }
    }

    /**
     * 停止定时器
     */
    private stopTimers(): void {
        if (this.flushTimer) {
            clearInterval(this.flushTimer);
            this.flushTimer = null;
        }
        if (this.fsyncTimer) {
            clearInterval(this.fsyncTimer);
            this.fsyncTimer = null;
        }
    }

    /**
     * 序列化 WAL 记录
     */
    private serializeRecord(record: WalRecordEntry): Buffer {
        const keyLen = Buffer.byteLength(record.key, "utf8");
        const valueLen = record.value ? record.value.length : 0;
        const headerSize = 8 + 8 + 1 + 4 + 4;
        const totalSize = headerSize + keyLen + valueLen + 4;

        const writer = new OptimizedBufferWriter(totalSize);
        writer.writeBigUInt64LE(record.lsn);
        writer.writeBigUInt64LE(record.transactionId);
        writer.writeUInt8(record.operationType);
        writer.writeUInt32LE(keyLen);
        writer.writeUInt32LE(valueLen);
        writer.writeStringRaw(record.key);

        if (record.value && valueLen > 0) {
            writer.writeBuffer(record.value);
        }

        const checksum = this.calculateChecksum(writer.data, 0, writer.position);
        writer.writeUInt32LE(checksum);

        return writer.data;
    }

    /**
     * 计算校验和
     */
    private calculateChecksum(buffer: Buffer, offset: number, length: number): number {
        let crc = 0xffffffff;
        const table = this.getCrc32Table();

        for (let i = offset; i < offset + length; i++) {
            const byte = buffer[i]!;
            crc = (crc >>> 8) ^ table[(crc ^ byte) & 0xff]!;
        }

        return (crc ^ 0xffffffff) >>> 0;
    }

    /**
     * 获取 CRC32 表
     */
    private crc32Table: Uint32Array | null = null;
    private getCrc32Table(): Uint32Array {
        if (!this.crc32Table) {
            this.crc32Table = new Uint32Array(256);
            for (let i = 0; i < 256; i++) {
                let crc = i;
                for (let j = 0; j < 8; j++) {
                    crc = crc & 1 ? 0xedb88320 ^ (crc >>> 1) : crc >>> 1;
                }
                this.crc32Table[i] = crc;
            }
        }
        return this.crc32Table;
    }

    /**
     * 添加记录到队列
     * @param record WAL 记录
     */
    public async append(record: WalRecordEntry): Promise<void> {
        if (this.closed) {
            throw new Error("BatchWALWriter is closed");
        }

        const buffer = this.serializeRecord(record);

        if (this.queue.length >= this.queueCapacity) {
            await this.flush();
        }

        this.queue.push({ record, buffer });

        if (this.queue.length >= this.batchSize) {
            await this.flush();
        }
    }

    /**
     * 批量添加记录
     * @param records WAL 记录数组
     */
    public async appendMany(records: WalRecordEntry[]): Promise<void> {
        for (const record of records) {
            await this.append(record);
        }
    }

    /**
     * 刷新队列到磁盘
     */
    public async flush(): Promise<BatchFlushResult> {
        if (this.closed) {
            return {
                recordsWritten: 0,
                bytesWritten: 0,
                durationMs: 0,
                fsynced: false,
            };
        }

        if (this.isFlushing) {
            this.pendingFlush = true;
            return {
                recordsWritten: 0,
                bytesWritten: 0,
                durationMs: 0,
                fsynced: false,
            };
        }

        if (this.queue.length === 0) {
            return {
                recordsWritten: 0,
                bytesWritten: 0,
                durationMs: 0,
                fsynced: false,
            };
        }

        this.isFlushing = true;
        const startTime = Date.now();

        try {
            if (!this.handle) {
                await this.open();
            }

            const entries = this.queue.splice(0, this.queue.length);
            const concatenator = new BufferConcatenator();

            for (const entry of entries) {
                concatenator.append(entry.buffer);
            }

            const buffer = concatenator.concat();
            const bytesWritten = buffer.length;

            await this.handle!.write(buffer, 0, buffer.length, Number(this.currentOffset));

            this.currentOffset += BigInt(bytesWritten);
            this.totalWritten += entries.length;
            this.writeCountSinceFsync += entries.length;
            this.lastFlushTime = Date.now();

            let fsynced = false;
            if (this.fsyncStrategy === "always") {
                await this.fsync();
                fsynced = true;
            } else if (this.fsyncStrategy === "batch" && this.writeCountSinceFsync >= this.fsyncBatchSize) {
                await this.fsync();
                fsynced = true;
            }

            const durationMs = Date.now() - startTime;

            return {
                recordsWritten: entries.length,
                bytesWritten,
                durationMs,
                fsynced,
            };
        } finally {
            this.isFlushing = false;

            if (this.pendingFlush) {
                this.pendingFlush = false;
                this.flush().catch(() => {});
            }
        }
    }

    /**
     * 同步到磁盘
     */
    public async fsync(): Promise<void> {
        if (this.closed || !this.handle) {
            return;
        }

        await this.handle.sync();
        this.writeCountSinceFsync = 0;
        this.lastFsyncTime = Date.now();
    }

    /**
     * 获取统计信息
     */
    public getStats(): BatchWriteStats {
        return {
            pendingCount: this.queue.length,
            writtenCount: this.totalWritten,
            lastFlushTime: this.lastFlushTime,
            lastFsyncTime: this.lastFsyncTime,
            queueCapacity: this.queueCapacity,
        };
    }

    /**
     * 获取当前偏移量
     */
    public getCurrentOffset(): bigint {
        return this.currentOffset;
    }

    /**
     * 获取文件路径
     */
    public getFilePath(): string {
        return this.filePath;
    }

    /**
     * 清空队列
     */
    public clearQueue(): void {
        this.queue = [];
    }

    /**
     * 关闭写入器
     */
    public async close(): Promise<void> {
        if (this.closed) {
            return;
        }

        this.closed = true;
        this.stopTimers();

        await this.flush();

        if (this.fsyncStrategy !== "manual") {
            await this.fsync();
        }

        if (this.handle) {
            await this.handle.close();
            this.handle = null;
        }
    }

    /**
     * 清空文件
     */
    public async clear(): Promise<void> {
        await this.close();

        await fs.truncate(this.filePath, 0);

        this.queue = [];
        this.currentOffset = 0n;
        this.writeCountSinceFsync = 0;
        this.totalWritten = 0;
        this.closed = false;

        await this.open();
    }
}

/**
 * 批量写入管理器
 * 提供更高层的批量写入接口
 */
export class BatchWriteManager {
    private writers: Map<string, BatchWalWriter>;
    private defaultOptions: BatchWriteOptions;

    /**
     * 创建批量写入管理器
     * @param defaultOptions 默认配置
     */
    constructor(defaultOptions: BatchWriteOptions = {}) {
        this.writers = new Map();
        this.defaultOptions = defaultOptions;
    }

    /**
     * 获取或创建写入器
     * @param name 写入器名称
     * @param filePath 文件路径
     * @param options 配置选项
     */
    public async getWriter(name: string, filePath: string, options?: BatchWriteOptions): Promise<BatchWalWriter> {
        let writer = this.writers.get(name);

        if (!writer) {
            writer = new BatchWalWriter(filePath, {
                ...this.defaultOptions,
                ...options,
            });
            await writer.open();
            this.writers.set(name, writer);
        }

        return writer;
    }

    /**
     * 写入记录
     * @param name 写入器名称
     * @param record WAL 记录
     */
    public async append(name: string, record: WalRecordEntry): Promise<void> {
        const writer = this.writers.get(name);
        if (!writer) {
            throw new Error(`Writer "${name}" not found`);
        }
        await writer.append(record);
    }

    /**
     * 刷新指定写入器
     * @param name 写入器名称
     */
    public async flush(name: string): Promise<BatchFlushResult> {
        const writer = this.writers.get(name);
        if (!writer) {
            throw new Error(`Writer "${name}" not found`);
        }
        return writer.flush();
    }

    /**
     * 刷新所有写入器
     */
    public async flushAll(): Promise<Map<string, BatchFlushResult>> {
        const results = new Map<string, BatchFlushResult>();

        for (const [name, writer] of this.writers) {
            results.set(name, await writer.flush());
        }

        return results;
    }

    /**
     * 同步指定写入器
     * @param name 写入器名称
     */
    public async fsync(name: string): Promise<void> {
        const writer = this.writers.get(name);
        if (!writer) {
            throw new Error(`Writer "${name}" not found`);
        }
        await writer.fsync();
    }

    /**
     * 同步所有写入器
     */
    public async fsyncAll(): Promise<void> {
        const promises: Promise<void>[] = [];

        for (const writer of this.writers.values()) {
            promises.push(writer.fsync());
        }

        await Promise.all(promises);
    }

    /**
     * 关闭指定写入器
     * @param name 写入器名称
     */
    public async closeWriter(name: string): Promise<void> {
        const writer = this.writers.get(name);
        if (writer) {
            await writer.close();
            this.writers.delete(name);
        }
    }

    /**
     * 关闭所有写入器
     */
    public async closeAll(): Promise<void> {
        const promises: Promise<void>[] = [];

        for (const writer of this.writers.values()) {
            promises.push(writer.close());
        }

        await Promise.all(promises);
        this.writers.clear();
    }

    /**
     * 获取所有写入器名称
     */
    public getWriterNames(): string[] {
        return Array.from(this.writers.keys());
    }

    /**
     * 获取写入器统计信息
     * @param name 写入器名称
     */
    public getStats(name: string): BatchWriteStats | undefined {
        const writer = this.writers.get(name);
        return writer?.getStats();
    }

    /**
     * 获取所有写入器统计信息
     */
    public getAllStats(): Map<string, BatchWriteStats> {
        const stats = new Map<string, BatchWriteStats>();

        for (const [name, writer] of this.writers) {
            stats.set(name, writer.getStats());
        }

        return stats;
    }
}
