/**
 * .light 文件格式实现
 * 主数据文件，存储数据库的完整快照
 */

import { LIGHT_MAGIC, LIGHT_VERSION, DEFAULT_PAGE_SIZE, HEADER_SIZE, PAGE_HEADER_SIZE, FILE_EXTENSIONS } from "../constants";
import { LightHeader, Page, Record, PageMap, RecordMap } from "../types";
import { BinaryUtils, BufferWriter, BufferReader, FileUtils, FileHandleWrapper } from "../utils";
import { FileFormatError, PageError } from "../errors";

/**
 * Light 文件序列化器
 * 负责将数据序列化为 .light 文件格式
 */
export class LightFileSerializer {
    private readonly pageSize: number;

    /**
     * 创建序列化器
     * @param pageSize 页面大小，默认 4KB
     */
    constructor(pageSize: number = DEFAULT_PAGE_SIZE) {
        this.pageSize = pageSize;
    }

    /**
     * 序列化文件头部
     * @param header 头部数据
     */
    public serializeHeader(header: LightHeader): Buffer {
        const writer = new BufferWriter(HEADER_SIZE);
        writer.writeUInt32LE(header.magic);
        writer.writeUInt32LE(header.version);
        writer.writeUInt32LE(header.pageSize);
        writer.writeBigUInt64LE(header.lastSnapshotLsn);
        writer.writeUInt32LE(0);
        writer.writeUInt32LE(0);
        writer.writeUInt32LE(0);
        writer.writeUInt32LE(0);
        return writer.data;
    }

    /**
     * 反序列化文件头部
     * @param buffer 包含头部数据的 Buffer
     */
    public deserializeHeader(buffer: Buffer): LightHeader {
        if (buffer.length < HEADER_SIZE) {
            throw new FileFormatError("Invalid header size", "", `at least ${HEADER_SIZE} bytes`, buffer.length);
        }

        const reader = new BufferReader(buffer);
        const magic = reader.readUInt32LE();
        const version = reader.readUInt32LE();
        const pageSize = reader.readUInt32LE();
        const lastSnapshotLsn = reader.readBigUInt64LE();

        if (magic !== LIGHT_MAGIC) {
            throw new FileFormatError(`Invalid magic number: expected ${LIGHT_MAGIC}, got ${magic}`, "", LIGHT_MAGIC.toString(), magic);
        }

        return {
            magic,
            version,
            pageSize,
            lastSnapshotLsn,
        };
    }

    /**
     * 序列化单条记录
     * @param record 记录数据
     */
    public serializeRecord(record: Record): Buffer {
        const keyLength = BinaryUtils.stringByteLength(record.key);
        const valueLength = record.value.length;
        const recordSize = 4 + keyLength + 4 + valueLength + 4;

        const writer = new BufferWriter(recordSize);
        writer.writeUInt32LE(keyLength);
        writer.writeString(record.key);
        writer.writeUInt32LE(valueLength);
        writer.writeBuffer(record.value);
        writer.writeChecksum(0);

        return writer.data;
    }

    /**
     * 反序列化单条记录
     * @param buffer 包含记录数据的 Buffer
     * @param startOffset 起始偏移量
     */
    public deserializeRecord(
        buffer: Buffer,
        startOffset: number = 0,
    ): {
        record: Record;
        bytesRead: number;
    } {
        const reader = new BufferReader(buffer, startOffset);

        const key = reader.readString();
        const valueLength = reader.readUInt32LE();
        const value = reader.readBuffer(valueLength);
        reader.readAndVerifyChecksum(startOffset);

        return {
            record: { key, value },
            bytesRead: reader.position - startOffset,
        };
    }

    /**
     * 序列化页面
     * @param page 页面数据
     */
    public serializePage(page: Page): Buffer {
        const pageData: Buffer[] = [];

        const recordBuffers = page.records.map((r) => this.serializeRecord(r));
        const recordsData = BinaryUtils.concat(recordBuffers);

        const headerWriter = new BufferWriter(PAGE_HEADER_SIZE);
        headerWriter.writeUInt32LE(page.pageId);
        headerWriter.writeUInt16LE(page.records.length);
        headerWriter.writeUInt16LE(this.pageSize - PAGE_HEADER_SIZE - recordsData.length);

        pageData.push(headerWriter.data);
        pageData.push(recordsData);

        const pageBuffer = BinaryUtils.concat(pageData);
        const checksum = BinaryUtils.calculateChecksum(pageBuffer);

        const finalWriter = new BufferWriter(this.pageSize);
        finalWriter.writeBuffer(pageBuffer);
        finalWriter.writeUInt32LE(checksum);

        const padding = this.pageSize - finalWriter.position;
        if (padding > 0) {
            finalWriter.writeBuffer(Buffer.alloc(padding));
        }

        return finalWriter.data;
    }

    /**
     * 反序列化页面
     * @param buffer 包含页面数据的 Buffer
     * @param pageId 页面ID
     */
    public deserializePage(buffer: Buffer, pageId: number): Page {
        if (buffer.length < this.pageSize) {
            throw new PageError(`Invalid page size: expected ${this.pageSize}, got ${buffer.length}`, pageId);
        }

        const checksum = buffer.readUInt32LE(this.pageSize - 4);
        BinaryUtils.verifyChecksum(buffer, checksum, 0, this.pageSize - 4);

        const reader = new BufferReader(buffer);
        const readPageId = reader.readUInt32LE();
        const recordCount = reader.readUInt16LE();

        if (readPageId !== pageId) {
            throw new PageError(`Page ID mismatch: expected ${pageId}, got ${readPageId}`, pageId);
        }

        const records: Record[] = [];
        for (let i = 0; i < recordCount; i++) {
            const { record } = this.deserializeRecord(buffer, reader.position);
            records.push(record);
        }

        return {
            pageId,
            records,
            checksum,
        };
    }

    /**
     * 序列化整个数据库快照
     * @param pages 页面映射
     * @param lastSnapshotLsn 上次快照的 LSN
     */
    public serializeSnapshot(pages: PageMap, lastSnapshotLsn: bigint): Buffer {
        const buffers: Buffer[] = [];

        const header: LightHeader = {
            magic: LIGHT_MAGIC,
            version: LIGHT_VERSION,
            pageSize: this.pageSize,
            lastSnapshotLsn,
        };
        buffers.push(this.serializeHeader(header));

        const sortedPageIds = Array.from(pages.keys()).sort((a, b) => a - b);
        for (const pageId of sortedPageIds) {
            const page = pages.get(pageId);
            if (page) {
                buffers.push(this.serializePage(page));
            }
        }

        return BinaryUtils.concat(buffers);
    }

    /**
     * 获取页面大小
     */
    public getPageSize(): number {
        return this.pageSize;
    }
}

/**
 * Light 文件管理器
 * 负责 .light 文件的读写操作
 */
export class LightFileManager {
    private readonly basePath: string;
    private readonly dbName: string;
    private readonly serializer: LightFileSerializer;
    private handle: FileHandleWrapper | null = null;

    /**
     * 创建文件管理器
     * @param basePath 数据库基础路径
     * @param dbName 数据库名称
     * @param pageSize 页面大小
     */
    constructor(basePath: string, dbName: string, pageSize: number = DEFAULT_PAGE_SIZE) {
        this.basePath = basePath;
        this.dbName = dbName;
        this.serializer = new LightFileSerializer(pageSize);
    }

    /**
     * 获取 .light 文件路径
     */
    public getFilePath(): string {
        return FileUtils.buildFilePath(this.basePath, this.dbName, FILE_EXTENSIONS.LIGHT);
    }

    /**
     * 获取临时文件路径
     */
    private getTempFilePath(): string {
        return FileUtils.buildFilePath(this.basePath, this.dbName, FILE_EXTENSIONS.TEMP);
    }

    /**
     * 打开文件
     */
    public async open(): Promise<void> {
        const filePath = this.getFilePath();
        const exists = await FileUtils.exists(filePath);

        if (exists) {
            this.handle = new FileHandleWrapper(filePath, "r+");
        } else {
            this.handle = new FileHandleWrapper(filePath, "w+");
        }
        await this.handle.open();
    }

    /**
     * 关闭文件
     */
    public async close(): Promise<void> {
        if (this.handle) {
            await this.handle.close();
            this.handle = null;
        }
    }

    /**
     * 读取文件头部
     */
    public async readHeader(): Promise<LightHeader | null> {
        if (!this.handle) {
            throw new FileFormatError("File not open", this.getFilePath());
        }

        const size = await this.handle.size();
        if (size < HEADER_SIZE) {
            return null;
        }

        const buffer = BinaryUtils.alloc(HEADER_SIZE);
        await this.handle.read(buffer, 0, HEADER_SIZE, 0);

        return this.serializer.deserializeHeader(buffer);
    }

    /**
     * 写入文件头部
     * @param header 头部数据
     */
    public async writeHeader(header: LightHeader): Promise<void> {
        if (!this.handle) {
            throw new FileFormatError("File not open", this.getFilePath());
        }

        const buffer = this.serializer.serializeHeader(header);
        await this.handle.write(buffer, 0, buffer.length, 0);
    }

    /**
     * 读取指定页面
     * @param pageId 页面ID
     */
    public async readPage(pageId: number): Promise<Page | null> {
        if (!this.handle) {
            throw new FileFormatError("File not open", this.getFilePath());
        }

        const pageSize = this.serializer.getPageSize();
        const position = HEADER_SIZE + pageId * pageSize;
        const size = await this.handle.size();

        if (position + pageSize > size) {
            return null;
        }

        const buffer = BinaryUtils.alloc(pageSize);
        await this.handle.read(buffer, 0, pageSize, position);

        return this.serializer.deserializePage(buffer, pageId);
    }

    /**
     * 写入指定页面
     * @param page 页面数据
     */
    public async writePage(page: Page): Promise<void> {
        if (!this.handle) {
            throw new FileFormatError("File not open", this.getFilePath());
        }

        const pageSize = this.serializer.getPageSize();
        const position = HEADER_SIZE + page.pageId * pageSize;
        const buffer = this.serializer.serializePage(page);

        await this.handle.write(buffer, 0, buffer.length, position);
    }

    /**
     * 加载所有页面
     */
    public async loadAllPages(): Promise<PageMap> {
        if (!this.handle) {
            throw new FileFormatError("File not open", this.getFilePath());
        }

        const pages: PageMap = new Map();
        const pageSize = this.serializer.getPageSize();
        const size = await this.handle.size();

        if (size <= HEADER_SIZE) {
            return pages;
        }

        const pageCount = Math.floor((size - HEADER_SIZE) / pageSize);

        for (let pageId = 0; pageId < pageCount; pageId++) {
            const page = await this.readPage(pageId);
            if (page) {
                pages.set(pageId, page);
            }
        }

        return pages;
    }

    /**
     * 加载所有记录到内存映射
     */
    public async loadAllRecords(): Promise<RecordMap> {
        const pages = await this.loadAllPages();
        const records: RecordMap = new Map();

        for (const page of pages.values()) {
            for (const record of page.records) {
                records.set(record.key, record.value);
            }
        }

        return records;
    }

    /**
     * 原子性写入完整快照
     * 先写临时文件，再 rename 确保原子性
     * @param pages 页面映射
     * @param lastSnapshotLsn 上次快照的 LSN
     */
    public async writeSnapshot(pages: PageMap, lastSnapshotLsn: bigint): Promise<void> {
        const tempPath = this.getTempFilePath();
        const finalPath = this.getFilePath();

        const data = this.serializer.serializeSnapshot(pages, lastSnapshotLsn);

        await FileUtils.writeFile(tempPath, data);
        await FileUtils.syncFile(tempPath);
        await FileUtils.rename(tempPath, finalPath);

        if (this.handle) {
            await this.handle.close();
            this.handle = new FileHandleWrapper(finalPath, "r+");
            await this.handle.open();
        }
    }

    /**
     * 获取文件大小
     */
    public async getFileSize(): Promise<number> {
        if (!this.handle) {
            return 0;
        }
        return this.handle.size();
    }

    /**
     * 同步到磁盘
     */
    public async sync(): Promise<void> {
        if (this.handle) {
            await this.handle.sync();
        }
    }

    /**
     * 检查文件是否存在
     */
    public async exists(): Promise<boolean> {
        return FileUtils.exists(this.getFilePath());
    }

    /**
     * 获取页面大小
     */
    public getPageSize(): number {
        return this.serializer.getPageSize();
    }
}

/**
 * 页面管理器
 * 管理内存中的页面分配和回收
 */
export class PageManager {
    private readonly pageSize: number;
    private pages: PageMap;
    private nextPageId: number;
    private dirtyPages: Set<number>;

    /**
     * 创建页面管理器
     * @param pageSize 页面大小
     */
    constructor(pageSize: number = DEFAULT_PAGE_SIZE) {
        this.pageSize = pageSize;
        this.pages = new Map();
        this.nextPageId = 0;
        this.dirtyPages = new Set();
    }

    /**
     * 创建新页面
     */
    public createPage(): Page {
        const pageId = this.nextPageId++;
        const page: Page = {
            pageId,
            records: [],
            checksum: 0,
        };
        this.pages.set(pageId, page);
        this.dirtyPages.add(pageId);
        return page;
    }

    /**
     * 获取页面
     * @param pageId 页面ID
     */
    public getPage(pageId: number): Page | undefined {
        return this.pages.get(pageId);
    }

    /**
     * 获取或创建页面
     * @param pageId 页面ID
     */
    public getOrCreatePage(pageId: number): Page {
        let page = this.pages.get(pageId);
        if (!page) {
            page = {
                pageId,
                records: [],
                checksum: 0,
            };
            this.pages.set(pageId, page);
            if (pageId >= this.nextPageId) {
                this.nextPageId = pageId + 1;
            }
        }
        return page;
    }

    /**
     * 标记页面为脏页
     * @param pageId 页面ID
     */
    public markDirty(pageId: number): void {
        this.dirtyPages.add(pageId);
    }

    /**
     * 获取所有脏页
     */
    public getDirtyPages(): Page[] {
        const dirty: Page[] = [];
        for (const pageId of this.dirtyPages) {
            const page = this.pages.get(pageId);
            if (page) {
                dirty.push(page);
            }
        }
        return dirty;
    }

    /**
     * 清除脏页标记
     */
    public clearDirtyFlags(): void {
        this.dirtyPages.clear();
    }

    /**
     * 加载页面数据
     * @param pages 页面映射
     */
    public loadPages(pages: PageMap): void {
        this.pages = new Map(pages);
        this.nextPageId = 0;
        for (const pageId of pages.keys()) {
            if (pageId >= this.nextPageId) {
                this.nextPageId = pageId + 1;
            }
        }
        this.dirtyPages.clear();
    }

    /**
     * 获取所有页面
     */
    public getAllPages(): PageMap {
        return new Map(this.pages);
    }

    /**
     * 获取页面数量
     */
    public getPageCount(): number {
        return this.pages.size;
    }

    /**
     * 计算记录在页面中的大小
     * @param record 记录
     */
    public calculateRecordSize(record: Record): number {
        const keyLength = BinaryUtils.stringByteLength(record.key);
        return 4 + keyLength + 4 + record.value.length + 4;
    }

    /**
     * 获取页面可用空间
     * @param page 页面
     */
    public getAvailableSpace(page: Page): number {
        const usedSpace = page.records.reduce((sum, r) => sum + this.calculateRecordSize(r), PAGE_HEADER_SIZE + 4);
        return this.pageSize - usedSpace;
    }

    /**
     * 查找可以容纳记录的页面
     * @param record 记录
     */
    public findPageForRecord(record: Record): Page | null {
        const recordSize = this.calculateRecordSize(record);

        for (const page of this.pages.values()) {
            if (this.getAvailableSpace(page) >= recordSize) {
                return page;
            }
        }

        return null;
    }

    /**
     * 获取下一个页面ID
     */
    public getNextPageId(): number {
        return this.nextPageId;
    }

    /**
     * 获取页面大小
     */
    public getPageSize(): number {
        return this.pageSize;
    }
}
