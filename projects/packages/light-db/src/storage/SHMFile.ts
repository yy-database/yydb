/**
 * .shm 文件管理实现
 * 存储 WAL 索引，用于加速恢复时构建脏页表
 */

import { SHM_HEADER_SIZE, SHM_ENTRY_SIZE, SHM_DEFAULT_CAPACITY, FILE_EXTENSIONS } from "../constants";
import { ShmHeader, ShmEntry } from "../types";
import { BinaryUtils, BufferWriter, BufferReader, FileUtils } from "../utils";
import { FileFormatError, ChecksumError } from "../errors";

/**
 * SHM 文件序列化器
 * 负责 SHM 数据的二进制序列化
 */
export class ShmSerializer {
    /**
     * 序列化 SHM 头部
     * @param header 头部数据
     */
    public serializeHeader(header: ShmHeader): Buffer {
        const writer = new BufferWriter(SHM_HEADER_SIZE);
        writer.writeBigUInt64LE(header.walFileSize);
        writer.writeBigUInt64LE(header.lastCompleteLsn);
        writer.writeUInt32LE(header.recordCount);
        writer.writeUInt32LE(header.checksum);
        return writer.data;
    }

    /**
     * 反序列化 SHM 头部
     * @param buffer 包含头部数据的 Buffer
     */
    public deserializeHeader(buffer: Buffer): ShmHeader {
        if (buffer.length < SHM_HEADER_SIZE) {
            throw new FileFormatError("Invalid SHM header size", "", `at least ${SHM_HEADER_SIZE} bytes`, buffer.length);
        }

        const reader = new BufferReader(buffer);
        const walFileSize = reader.readBigUInt64LE();
        const lastCompleteLsn = reader.readBigUInt64LE();
        const recordCount = reader.readUInt32LE();
        const checksum = reader.readUInt32LE();

        const calculatedChecksum = BinaryUtils.calculateChecksum(buffer, 0, SHM_HEADER_SIZE - 4);
        if (calculatedChecksum !== checksum) {
            throw new ChecksumError("SHM header checksum verification failed", checksum, calculatedChecksum);
        }

        return {
            walFileSize,
            lastCompleteLsn,
            recordCount,
            checksum,
        };
    }

    /**
     * 序列化 SHM 条目
     * @param entry 条目数据
     */
    public serializeEntry(entry: ShmEntry): Buffer {
        const writer = new BufferWriter(SHM_ENTRY_SIZE);
        writer.writeUInt32LE(entry.pageId);
        writer.writeBigUInt64LE(entry.lsn);
        writer.writeBigUInt64LE(entry.walOffset);
        return writer.data;
    }

    /**
     * 反序列化 SHM 条目
     * @param buffer 包含条目数据的 Buffer
     * @param offset 起始偏移量
     */
    public deserializeEntry(buffer: Buffer, offset: number = 0): ShmEntry {
        const reader = new BufferReader(buffer, offset);
        const pageId = reader.readUInt32LE();
        const lsn = reader.readBigUInt64LE();
        const walOffset = reader.readBigUInt64LE();

        return {
            pageId,
            lsn,
            walOffset,
        };
    }

    /**
     * 计算哈希值
     * @param pageId 页面ID
     * @param capacity 哈希表容量
     */
    public hash(pageId: number, capacity: number): number {
        let hash = pageId;
        hash = ((hash >> 16) ^ hash) * 0x45d9f3b;
        hash = ((hash >> 16) ^ hash) * 0x45d9f3b;
        hash = (hash >> 16) ^ hash;
        return Math.abs(hash) % capacity;
    }
}

/**
 * SHM 哈希表
 * 内存中的 WAL 索引结构
 */
export class ShmHashTable {
    private entries: Map<number, ShmEntry>;
    private readonly capacity: number;

    /**
     * 创建哈希表
     * @param capacity 容量
     */
    constructor(capacity: number = SHM_DEFAULT_CAPACITY) {
        this.entries = new Map();
        this.capacity = capacity;
    }

    /**
     * 插入或更新条目
     * @param entry 条目数据
     */
    public set(entry: ShmEntry): void {
        const existing = this.entries.get(entry.pageId);
        if (!existing || entry.lsn > existing.lsn) {
            this.entries.set(entry.pageId, entry);
        }
    }

    /**
     * 获取条目
     * @param pageId 页面ID
     */
    public get(pageId: number): ShmEntry | undefined {
        return this.entries.get(pageId);
    }

    /**
     * 删除条目
     * @param pageId 页面ID
     */
    public delete(pageId: number): boolean {
        return this.entries.delete(pageId);
    }

    /**
     * 检查是否存在条目
     * @param pageId 页面ID
     */
    public has(pageId: number): boolean {
        return this.entries.has(pageId);
    }

    /**
     * 获取所有条目
     */
    public getAll(): ShmEntry[] {
        return Array.from(this.entries.values());
    }

    /**
     * 获取条目数量
     */
    public size(): number {
        return this.entries.size;
    }

    /**
     * 清空哈希表
     */
    public clear(): void {
        this.entries.clear();
    }

    /**
     * 获取容量
     */
    public getCapacity(): number {
        return this.capacity;
    }

    /**
     * 迭代器
     */
    public [Symbol.iterator](): Iterator<ShmEntry> {
        return this.entries.values()[Symbol.iterator]();
    }
}

/**
 * SHM 文件管理器
 * 负责 .shm 文件的读写操作
 */
export class ShmFileManager {
    private readonly filePath: string;
    private readonly serializer: ShmSerializer;
    private hashTable: ShmHashTable;
    private header: ShmHeader;

    /**
     * 创建 SHM 文件管理器
     * @param basePath 数据库基础路径
     * @param dbName 数据库名称
     * @param capacity 哈希表容量
     */
    constructor(basePath: string, dbName: string, capacity: number = SHM_DEFAULT_CAPACITY) {
        this.filePath = FileUtils.buildFilePath(basePath, dbName, FILE_EXTENSIONS.SHM);
        this.serializer = new ShmSerializer();
        this.hashTable = new ShmHashTable(capacity);
        this.header = {
            walFileSize: 0n,
            lastCompleteLsn: 0n,
            recordCount: 0,
            checksum: 0,
        };
    }

    /**
     * 加载 SHM 文件
     */
    public async load(): Promise<boolean> {
        const exists = await FileUtils.exists(this.filePath);
        if (!exists) {
            return false;
        }

        try {
            const data = await FileUtils.readFile(this.filePath);

            if (data.length < SHM_HEADER_SIZE) {
                return false;
            }

            this.header = this.serializer.deserializeHeader(data);

            const entryCount = Math.floor((data.length - SHM_HEADER_SIZE) / SHM_ENTRY_SIZE);
            for (let i = 0; i < entryCount; i++) {
                const offset = SHM_HEADER_SIZE + i * SHM_ENTRY_SIZE;
                const entry = this.serializer.deserializeEntry(data, offset);
                this.hashTable.set(entry);
            }

            return true;
        } catch (error) {
            return false;
        }
    }

    /**
     * 保存 SHM 文件
     */
    public async save(): Promise<void> {
        const entries = this.hashTable.getAll();

        this.header.recordCount = entries.length;
        const headerBuffer = this.serializer.serializeHeader(this.header);
        const checksum = BinaryUtils.calculateChecksum(headerBuffer, 0, SHM_HEADER_SIZE - 4);
        headerBuffer.writeUInt32LE(checksum, SHM_HEADER_SIZE - 4);

        const entryBuffers = entries.map((e) => this.serializer.serializeEntry(e));
        const allBuffers = [headerBuffer, ...entryBuffers];
        const data = BinaryUtils.concat(allBuffers);

        await FileUtils.writeFile(this.filePath, data);
    }

    /**
     * 更新 WAL 索引条目
     * @param pageId 页面ID
     * @param lsn LSN
     * @param walOffset WAL 偏移量
     */
    public updateEntry(pageId: number, lsn: bigint, walOffset: bigint): void {
        this.hashTable.set({ pageId, lsn, walOffset });
    }

    /**
     * 获取页面的 WAL 索引
     * @param pageId 页面ID
     */
    public getEntry(pageId: number): ShmEntry | undefined {
        return this.hashTable.get(pageId);
    }

    /**
     * 获取所有条目
     */
    public getAllEntries(): ShmEntry[] {
        return this.hashTable.getAll();
    }

    /**
     * 更新头部信息
     * @param walFileSize WAL 文件大小
     * @param lastCompleteLsn 最后完整页的 LSN
     */
    public updateHeader(walFileSize: bigint, lastCompleteLsn: bigint): void {
        this.header.walFileSize = walFileSize;
        this.header.lastCompleteLsn = lastCompleteLsn;
    }

    /**
     * 获取头部信息
     */
    public getHeader(): ShmHeader {
        return { ...this.header };
    }

    /**
     * 清空索引
     */
    public async clear(): Promise<void> {
        this.hashTable.clear();
        this.header = {
            walFileSize: 0n,
            lastCompleteLsn: 0n,
            recordCount: 0,
            checksum: 0,
        };

        const exists = await FileUtils.exists(this.filePath);
        if (exists) {
            await FileUtils.deleteFile(this.filePath);
        }
    }

    /**
     * 获取条目数量
     */
    public getEntryCount(): number {
        return this.hashTable.size();
    }

    /**
     * 获取最后完整页的 LSN
     */
    public getLastCompleteLsn(): bigint {
        return this.header.lastCompleteLsn;
    }

    /**
     * 获取 WAL 文件大小
     */
    public getWalFileSize(): bigint {
        return this.header.walFileSize;
    }

    /**
     * 检查文件是否存在
     */
    public async exists(): Promise<boolean> {
        return FileUtils.exists(this.filePath);
    }

    /**
     * 获取文件路径
     */
    public getFilePath(): string {
        return this.filePath;
    }

    /**
     * 重建索引
     * 从 WAL 记录重建 SHM 索引
     * @param records WAL 记录数组
     * @param getOffsetByLsn 根据 LSN 获取偏移量的函数
     */
    public rebuildIndex(records: Array<{ lsn: bigint; key: string }>, getOffsetByLsn: (lsn: bigint) => number): void {
        this.hashTable.clear();

        for (const record of records) {
            const pageId = this.keyToPageId(record.key);
            const walOffset = BigInt(getOffsetByLsn(record.lsn));
            this.hashTable.set({
                pageId,
                lsn: record.lsn,
                walOffset,
            });
        }

        if (records.length > 0) {
            const lastRecord = records[records.length - 1]!;
            this.header.lastCompleteLsn = lastRecord.lsn;
        }
    }

    /**
     * 将键转换为页面ID
     * 简单的哈希函数
     * @param key 键
     */
    private keyToPageId(key: string): number {
        let hash = 0;
        for (let i = 0; i < key.length; i++) {
            const char = key.charCodeAt(i);
            hash = (hash << 5) - hash + char;
            hash = hash & hash;
        }
        return Math.abs(hash);
    }
}

/**
 * WAL 索引管理器
 * 提供高级的 WAL 索引操作接口
 */
export class WalIndexManager {
    private readonly shmManager: ShmFileManager;

    /**
     * 创建 WAL 索引管理器
     * @param basePath 数据库基础路径
     * @param dbName 数据库名称
     */
    constructor(basePath: string, dbName: string) {
        this.shmManager = new ShmFileManager(basePath, dbName);
    }

    /**
     * 初始化索引
     */
    public async initialize(): Promise<void> {
        const loaded = await this.shmManager.load();
        if (!loaded) {
            await this.shmManager.clear();
        }
    }

    /**
     * 记录 WAL 写入
     * @param pageId 页面ID
     * @param lsn LSN
     * @param walOffset WAL 偏移量
     */
    public async recordWrite(pageId: number, lsn: bigint, walOffset: bigint): Promise<void> {
        this.shmManager.updateEntry(pageId, lsn, walOffset);
        this.shmManager.updateHeader(BigInt(walOffset), lsn);
        await this.shmManager.save();
    }

    /**
     * 获取页面的最新 WAL 记录位置
     * @param pageId 页面ID
     */
    public getPageLocation(pageId: number): ShmEntry | undefined {
        return this.shmManager.getEntry(pageId);
    }

    /**
     * 获取需要回放的 WAL 范围
     */
    public getWalReplayRange(): { startLsn: bigint; endLsn: bigint } {
        const header = this.shmManager.getHeader();
        return {
            startLsn: header.lastCompleteLsn,
            endLsn: header.walFileSize,
        };
    }

    /**
     * 获取所有脏页
     */
    public getDirtyPages(): ShmEntry[] {
        return this.shmManager.getAllEntries();
    }

    /**
     * 检查点完成后重置索引
     * @param checkpointLsn 检查点 LSN
     */
    public async resetAfterCheckpoint(checkpointLsn: bigint): Promise<void> {
        await this.shmManager.clear();
        this.shmManager.updateHeader(0n, checkpointLsn);
        await this.shmManager.save();
    }

    /**
     * 同步索引到磁盘
     */
    public async sync(): Promise<void> {
        await this.shmManager.save();
    }

    /**
     * 关闭索引管理器
     */
    public async close(): Promise<void> {
        await this.shmManager.save();
    }

    /**
     * 获取 SHM 管理器
     */
    public getShmManager(): ShmFileManager {
        return this.shmManager;
    }
}
