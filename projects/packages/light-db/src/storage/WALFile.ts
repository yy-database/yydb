/**
 * WAL（预写日志）文件实现
 * 记录每一次写操作的增量日志，用于崩溃恢复与异步持久化
 */

import { OperationType, WAL_RECORD_HEADER_SIZE, FILE_EXTENSIONS } from "../constants";
import { WalRecord, WriteResult, BatchWriteResult, WalRecordIterator } from "../types";
import { BinaryUtils, BufferWriter, BufferReader, FileUtils, FileHandleWrapper } from "../utils";
import { WalError, ChecksumError } from "../errors";

/**
 * WAL 记录序列化器
 * 负责将 WAL 记录序列化为二进制格式
 */
export class WalRecordSerializer {
    /**
     * 序列化 WAL 记录
     * @param record WAL 记录
     */
    public serialize(record: WalRecord): Buffer {
        const keyLength = BinaryUtils.stringByteLength(record.key);
        const valueLength = record.value ? record.value.length : 0;
        const recordSize = WAL_RECORD_HEADER_SIZE + keyLength + valueLength;

        const writer = new BufferWriter(recordSize);
        writer.writeBigUInt64LE(record.lsn);
        writer.writeBigUInt64LE(record.transactionId);
        writer.writeUInt8(record.operationType);
        writer.writeUInt32LE(keyLength);
        writer.writeUInt32LE(valueLength);

        writer.writeStringRaw(record.key);

        if (record.value && valueLength > 0) {
            writer.writeBuffer(record.value);
        }

        writer.writeChecksum(0);

        return writer.data;
    }

    /**
     * 反序列化 WAL 记录
     * @param buffer 包含记录数据的 Buffer
     * @param startOffset 起始偏移量
     */
    public deserialize(
        buffer: Buffer,
        startOffset: number = 0,
    ): {
        record: WalRecord;
        bytesRead: number;
    } | null {
        const reader = new BufferReader(buffer, startOffset);

        if (!reader.hasRemaining(WAL_RECORD_HEADER_SIZE)) {
            return null;
        }

        const startPos = reader.position;

        const lsn = reader.readBigUInt64LE();
        const transactionId = reader.readBigUInt64LE();
        const operationType = reader.readUInt8() as OperationType;
        const keyLength = reader.readUInt32LE();
        const valueLength = reader.readUInt32LE();

        if (!reader.hasRemaining(keyLength + valueLength + 4)) {
            return null;
        }

        const key = BinaryUtils.readString(buffer, reader.position, keyLength);
        reader.position += keyLength;

        let value: Buffer | null = null;
        if (valueLength > 0) {
            value = reader.readBuffer(valueLength);
        }

        const checksum = reader.readUInt32LE();

        try {
            BinaryUtils.verifyChecksum(buffer, checksum, startPos, reader.position - startPos - 4);
        } catch (e) {
            if (e instanceof ChecksumError) {
                throw new WalError(`WAL record checksum verification failed at offset ${startOffset}`, lsn, startOffset);
            }
            throw e;
        }

        return {
            record: {
                lsn,
                transactionId,
                operationType,
                key,
                value,
                checksum,
            },
            bytesRead: reader.position - startPos,
        };
    }

    /**
     * 计算记录大小
     * @param key 键
     * @param value 值（可选）
     */
    public calculateRecordSize(key: string, value: Buffer | null): number {
        const keyLength = BinaryUtils.stringByteLength(key);
        const valueLength = value ? value.length : 0;
        return WAL_RECORD_HEADER_SIZE + keyLength + valueLength;
    }
}

/**
 * WAL 文件写入器
 * 负责将 WAL 记录追加写入文件
 */
export class WalWriter {
    private readonly filePath: string;
    private handle: FileHandleWrapper | null = null;
    private readonly serializer: WalRecordSerializer;
    private currentOffset: number;
    private pendingSync: boolean;
    private readonly syncStrategy: "always" | "batch" | "interval";
    private readonly batchSize: number;
    private writeCount: number;

    /**
     * 创建 WAL 写入器
     * @param basePath 数据库基础路径
     * @param dbName 数据库名称
     * @param syncStrategy 同步策略
     * @param batchSize 批量写入大小
     */
    constructor(basePath: string, dbName: string, syncStrategy: "always" | "batch" | "interval" = "always", batchSize: number = 100) {
        this.filePath = FileUtils.buildFilePath(basePath, dbName, FILE_EXTENSIONS.WAL);
        this.serializer = new WalRecordSerializer();
        this.currentOffset = 0;
        this.pendingSync = false;
        this.syncStrategy = syncStrategy;
        this.batchSize = batchSize;
        this.writeCount = 0;
    }

    /**
     * 打开 WAL 文件
     */
    public async open(): Promise<void> {
        const exists = await FileUtils.exists(this.filePath);

        if (exists) {
            this.handle = new FileHandleWrapper(this.filePath, "r+");
            await this.handle.open();
            this.currentOffset = await this.handle.size();
        } else {
            this.handle = new FileHandleWrapper(this.filePath, "w+");
            await this.handle.open();
            this.currentOffset = 0;
        }
    }

    /**
     * 关闭 WAL 文件
     */
    public async close(): Promise<void> {
        if (this.pendingSync && this.handle) {
            await this.handle.sync();
            this.pendingSync = false;
        }

        if (this.handle) {
            await this.handle.close();
            this.handle = null;
        }
    }

    /**
     * 写入单条 WAL 记录
     * @param record WAL 记录
     */
    public async write(record: WalRecord): Promise<WriteResult> {
        if (!this.handle) {
            throw new WalError("WAL file not open");
        }

        const buffer = this.serializer.serialize(record);
        const bytesWritten = await this.handle.write(buffer, 0, buffer.length, this.currentOffset);

        this.currentOffset += bytesWritten;
        this.writeCount++;
        this.pendingSync = true;

        if (this.syncStrategy === "always") {
            await this.sync();
        } else if (this.syncStrategy === "batch" && this.writeCount >= this.batchSize) {
            await this.sync();
            this.writeCount = 0;
        }

        return {
            success: true,
            lsn: record.lsn,
            bytesWritten,
        };
    }

    /**
     * 批量写入 WAL 记录
     * @param records WAL 记录数组
     */
    public async writeBatch(records: WalRecord[]): Promise<BatchWriteResult> {
        if (records.length === 0) {
            return {
                success: true,
                startLsn: 0n,
                endLsn: 0n,
                totalBytesWritten: 0,
                recordsWritten: 0,
            };
        }

        if (!this.handle) {
            throw new WalError("WAL file not open");
        }

        const buffers = records.map((r) => this.serializer.serialize(r));
        const combinedBuffer = BinaryUtils.concat(buffers);

        const bytesWritten = await this.handle.write(combinedBuffer, 0, combinedBuffer.length, this.currentOffset);

        this.currentOffset += bytesWritten;
        this.pendingSync = true;

        if (this.syncStrategy === "always" || this.syncStrategy === "batch") {
            await this.sync();
        }

        return {
            success: true,
            startLsn: records[0]!.lsn,
            endLsn: records[records.length - 1]!.lsn,
            totalBytesWritten: bytesWritten,
            recordsWritten: records.length,
        };
    }

    /**
     * 同步到磁盘
     */
    public async sync(): Promise<void> {
        if (this.handle && this.pendingSync) {
            await this.handle.sync();
            this.pendingSync = false;
        }
    }

    /**
     * 获取当前文件大小
     */
    public async getFileSize(): Promise<number> {
        return this.currentOffset;
    }

    /**
     * 获取当前偏移量
     */
    public getCurrentOffset(): number {
        return this.currentOffset;
    }

    /**
     * 清空 WAL 文件
     */
    public async clear(): Promise<void> {
        if (this.handle) {
            await this.handle.close();
        }

        await FileUtils.truncateFile(this.filePath, 0);

        this.handle = new FileHandleWrapper(this.filePath, "w+");
        await this.handle.open();
        this.currentOffset = 0;
        this.pendingSync = false;
        this.writeCount = 0;
    }

    /**
     * 获取文件路径
     */
    public getFilePath(): string {
        return this.filePath;
    }
}

/**
 * WAL 文件读取器
 * 负责顺序读取 WAL 记录
 */
export class WalReader {
    private readonly filePath: string;
    private handle: FileHandleWrapper | null = null;
    private readonly serializer: WalRecordSerializer;
    private fileSize: number;
    private currentOffset: number;
    private readBufferSize: number;
    private readBuffer: Buffer;
    private bufferValidBytes: number;

    /**
     * 创建 WAL 读取器
     * @param basePath 数据库基础路径
     * @param dbName 数据库名称
     * @param readBufferSize 读取缓冲区大小，默认 64KB
     */
    constructor(basePath: string, dbName: string, readBufferSize: number = 64 * 1024) {
        this.filePath = FileUtils.buildFilePath(basePath, dbName, FILE_EXTENSIONS.WAL);
        this.serializer = new WalRecordSerializer();
        this.fileSize = 0;
        this.currentOffset = 0;
        this.readBufferSize = readBufferSize;
        this.readBuffer = BinaryUtils.alloc(readBufferSize);
        this.bufferValidBytes = 0;
    }

    /**
     * 打开 WAL 文件
     */
    public async open(): Promise<void> {
        const exists = await FileUtils.exists(this.filePath);
        if (!exists) {
            this.fileSize = 0;
            return;
        }

        this.handle = new FileHandleWrapper(this.filePath, "r");
        await this.handle.open();
        this.fileSize = await this.handle.size();
        this.currentOffset = 0;
        this.bufferValidBytes = 0;
    }

    /**
     * 关闭 WAL 文件
     */
    public async close(): Promise<void> {
        if (this.handle) {
            await this.handle.close();
            this.handle = null;
        }
    }

    /**
     * 填充读取缓冲区
     */
    private async fillBuffer(): Promise<boolean> {
        if (!this.handle) {
            return false;
        }

        if (this.currentOffset >= this.fileSize) {
            return false;
        }

        const remaining = this.fileSize - this.currentOffset;
        const toRead = Math.min(this.readBufferSize, remaining);

        const bytesRead = await this.handle.read(this.readBuffer, 0, toRead, this.currentOffset);

        this.bufferValidBytes = bytesRead;
        this.currentOffset += bytesRead;

        return bytesRead > 0;
    }

    /**
     * 读取下一条 WAL 记录
     */
    public async readNext(): Promise<WalRecord | null> {
        if (!this.handle && this.fileSize === 0) {
            return null;
        }

        if (this.bufferValidBytes === 0) {
            if (!(await this.fillBuffer())) {
                return null;
            }
        }

        const buffer = this.readBuffer.subarray(0, this.bufferValidBytes);
        const result = this.serializer.deserialize(buffer, 0);

        if (!result) {
            return null;
        }

        this.bufferValidBytes = 0;
        return result.record;
    }

    /**
     * 创建 WAL 记录迭代器
     */
    public async *iterate(): WalRecordIterator {
        await this.open();

        while (true) {
            const record = await this.readNext();
            if (!record) {
                break;
            }
            yield record;
        }

        await this.close();
    }

    /**
     * 从指定偏移量开始读取
     * @param offset 起始偏移量
     */
    public async seek(offset: number): Promise<void> {
        this.currentOffset = offset;
        this.bufferValidBytes = 0;
    }

    /**
     * 获取当前偏移量
     */
    public getCurrentOffset(): number {
        return this.currentOffset;
    }

    /**
     * 获取文件大小
     */
    public getFileSize(): number {
        return this.fileSize;
    }

    /**
     * 读取所有 WAL 记录
     */
    public async readAll(): Promise<WalRecord[]> {
        const records: WalRecord[] = [];

        await this.open();

        while (true) {
            const record = await this.readNext();
            if (!record) {
                break;
            }
            records.push(record);
        }

        await this.close();
        return records;
    }

    /**
     * 获取文件路径
     */
    public getFilePath(): string {
        return this.filePath;
    }
}

/**
 * WAL 管理器
 * 提供统一的 WAL 读写接口
 */
export class WalManager {
    private readonly basePath: string;
    private readonly dbName: string;
    private writer: WalWriter | null = null;
    private reader: WalReader | null = null;
    private currentLsn: bigint;
    private currentTransactionId: bigint;

    /**
     * 创建 WAL 管理器
     * @param basePath 数据库基础路径
     * @param dbName 数据库名称
     */
    constructor(basePath: string, dbName: string) {
        this.basePath = basePath;
        this.dbName = dbName;
        this.currentLsn = 0n;
        this.currentTransactionId = 0n;
    }

    /**
     * 初始化 WAL 管理器
     */
    public async initialize(): Promise<void> {
        this.writer = new WalWriter(this.basePath, this.dbName);
        await this.writer.open();

        this.reader = new WalReader(this.basePath, this.dbName);
        await this.reader.open();

        const lastLsn = await this.getLastLsn();
        this.currentLsn = lastLsn + 1n;
        this.currentTransactionId = 0n;
    }

    /**
     * 获取最后一条记录的 LSN
     */
    public async getLastLsn(): Promise<bigint> {
        if (!this.reader) {
            return 0n;
        }

        let lastLsn = 0n;
        const records = await this.reader.readAll();

        for (const record of records) {
            if (record.lsn > lastLsn) {
                lastLsn = record.lsn;
            }
        }

        return lastLsn;
    }

    /**
     * 分配新的 LSN
     */
    public allocateLsn(): bigint {
        const lsn = this.currentLsn;
        this.currentLsn++;
        return lsn;
    }

    /**
     * 分配新的事务 ID
     */
    public allocateTransactionId(): bigint {
        const txId = this.currentTransactionId;
        this.currentTransactionId++;
        return txId;
    }

    /**
     * 写入插入记录
     * @param key 键
     * @param value 值
     */
    public async writeInsert(key: string, value: Buffer): Promise<WriteResult> {
        if (!this.writer) {
            throw new WalError("WAL manager not initialized");
        }

        const record: WalRecord = {
            lsn: this.allocateLsn(),
            transactionId: this.allocateTransactionId(),
            operationType: OperationType.Insert,
            key,
            value,
            checksum: 0,
        };

        return this.writer.write(record);
    }

    /**
     * 写入更新记录
     * @param key 键
     * @param value 新值
     */
    public async writeUpdate(key: string, value: Buffer): Promise<WriteResult> {
        if (!this.writer) {
            throw new WalError("WAL manager not initialized");
        }

        const record: WalRecord = {
            lsn: this.allocateLsn(),
            transactionId: this.allocateTransactionId(),
            operationType: OperationType.Update,
            key,
            value,
            checksum: 0,
        };

        return this.writer.write(record);
    }

    /**
     * 写入删除记录
     * @param key 键
     */
    public async writeDelete(key: string): Promise<WriteResult> {
        if (!this.writer) {
            throw new WalError("WAL manager not initialized");
        }

        const record: WalRecord = {
            lsn: this.allocateLsn(),
            transactionId: this.allocateTransactionId(),
            operationType: OperationType.Delete,
            key,
            value: null,
            checksum: 0,
        };

        return this.writer.write(record);
    }

    /**
     * 写入检查点标记
     */
    public async writeCheckpoint(): Promise<WriteResult> {
        if (!this.writer) {
            throw new WalError("WAL manager not initialized");
        }

        const record: WalRecord = {
            lsn: this.allocateLsn(),
            transactionId: this.allocateTransactionId(),
            operationType: OperationType.Checkpoint,
            key: "",
            value: null,
            checksum: 0,
        };

        return this.writer.write(record);
    }

    /**
     * 同步 WAL 到磁盘
     */
    public async sync(): Promise<void> {
        if (this.writer) {
            await this.writer.sync();
        }
    }

    /**
     * 清空 WAL 文件
     */
    public async clear(): Promise<void> {
        if (this.writer) {
            await this.writer.clear();
        }
        this.currentLsn = 1n;
    }

    /**
     * 获取 WAL 文件大小
     */
    public async getFileSize(): Promise<number> {
        if (!this.writer) {
            return 0;
        }
        return this.writer.getFileSize();
    }

    /**
     * 创建记录迭代器
     */
    public async *iterateRecords(): WalRecordIterator {
        if (!this.reader) {
            return;
        }

        const reader = new WalReader(this.basePath, this.dbName);
        yield* reader.iterate();
    }

    /**
     * 读取所有记录
     */
    public async readAllRecords(): Promise<WalRecord[]> {
        const reader = new WalReader(this.basePath, this.dbName);
        return reader.readAll();
    }

    /**
     * 关闭 WAL 管理器
     */
    public async close(): Promise<void> {
        if (this.writer) {
            await this.writer.close();
            this.writer = null;
        }

        if (this.reader) {
            await this.reader.close();
            this.reader = null;
        }
    }

    /**
     * 获取当前 LSN
     */
    public getCurrentLsn(): bigint {
        return this.currentLsn;
    }

    /**
     * 获取当前事务 ID
     */
    public getCurrentTransactionId(): bigint {
        return this.currentTransactionId;
    }

    /**
     * 设置当前 LSN（用于恢复）
     */
    public setCurrentLsn(lsn: bigint): void {
        this.currentLsn = lsn + 1n;
    }
}
