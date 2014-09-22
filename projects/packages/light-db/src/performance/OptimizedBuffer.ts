/**
 * 优化的二进制序列化工具
 * 使用 Buffer.allocUnsafe + 手动 fill，比 Buffer.alloc 快 2 倍以上
 */

import { BufferPoolOptions, BufferPoolStats } from "./types";

/**
 * Buffer 池
 * 预分配 Buffer 块，减少频繁内存分配
 */
export class BufferPool {
    private pool: Buffer[];
    private chunkSize: number;
    private maxPoolSize: number;
    private usedCount: number;
    private totalAllocated: number;
    private totalFreed: number;

    /**
     * 创建 Buffer 池
     * @param options 配置选项
     */
    constructor(options: BufferPoolOptions = {}) {
        this.chunkSize = options.chunkSize ?? 4096;
        this.maxPoolSize = options.poolSize ?? 256;
        this.pool = [];
        this.usedCount = 0;
        this.totalAllocated = 0;
        this.totalFreed = 0;

        if (options.preallocate !== false) {
            this.preallocate(Math.min(this.maxPoolSize, 16));
        }
    }

    /**
     * 预分配 Buffer 块
     * @param count 分配数量
     */
    private preallocate(count: number): void {
        for (let i = 0; i < count; i++) {
            this.pool.push(this.createChunk());
        }
    }

    /**
     * 创建新的 Buffer 块
     */
    private createChunk(): Buffer {
        return Buffer.allocUnsafe(this.chunkSize);
    }

    /**
     * 从池中获取 Buffer
     * @param size 需要的大小
     * @returns Buffer 实例
     */
    public allocate(size: number): Buffer {
        this.totalAllocated++;

        if (size <= this.chunkSize && this.pool.length > 0) {
            const buffer = this.pool.pop()!;
            this.usedCount++;
            return buffer.subarray(0, size);
        }

        return Buffer.allocUnsafe(size);
    }

    /**
     * 释放 Buffer 回池中
     * @param buffer 要释放的 Buffer
     */
    public release(buffer: Buffer): void {
        this.totalFreed++;

        if (buffer.length === this.chunkSize && this.pool.length < this.maxPoolSize) {
            buffer.fill(0);
            this.pool.push(buffer);
        }

        if (this.usedCount > 0) {
            this.usedCount--;
        }
    }

    /**
     * 获取统计信息
     */
    public getStats(): BufferPoolStats {
        return {
            totalChunks: this.pool.length + this.usedCount,
            availableChunks: this.pool.length,
            usedChunks: this.usedCount,
            totalMemory: (this.pool.length + this.usedCount) * this.chunkSize,
            usedMemory: this.usedCount * this.chunkSize,
        };
    }

    /**
     * 清空池
     */
    public clear(): void {
        this.pool = [];
        this.usedCount = 0;
    }

    /**
     * 获取块大小
     */
    public getChunkSize(): number {
        return this.chunkSize;
    }

    /**
     * 获取分配总数
     */
    public getTotalAllocated(): number {
        return this.totalAllocated;
    }

    /**
     * 获取释放总数
     */
    public getTotalFreed(): number {
        return this.totalFreed;
    }
}

/**
 * 全局 Buffer 池实例
 */
let globalPool: BufferPool | null = null;

/**
 * 获取全局 Buffer 池
 */
export function getGlobalBufferPool(): BufferPool {
    if (!globalPool) {
        globalPool = new BufferPool();
    }
    return globalPool;
}

/**
 * 优化的 Buffer 写入器
 * 使用 allocUnsafe + 手动 fill，性能更优
 */
export class OptimizedBufferWriter {
    private buffer: Buffer;
    private offset: number;
    private static pool: BufferPool = getGlobalBufferPool();

    /**
     * 创建优化的 Buffer 写入器
     * @param size 初始大小
     */
    constructor(size: number = 1024) {
        this.buffer = OptimizedBufferWriter.pool.allocate(size);
        this.offset = 0;
    }

    /**
     * 获取当前写入位置
     */
    public get position(): number {
        return this.offset;
    }

    /**
     * 获取已写入的数据
     */
    public get data(): Buffer {
        return this.buffer.subarray(0, this.offset);
    }

    /**
     * 获取剩余空间
     */
    public get remaining(): number {
        return this.buffer.length - this.offset;
    }

    /**
     * 确保有足够的空间
     * @param additional 需要的额外空间
     */
    private ensureCapacity(additional: number): void {
        const required = this.offset + additional;
        if (required > this.buffer.length) {
            const newSize = Math.max(required, this.buffer.length * 2);
            const newBuffer = Buffer.allocUnsafe(newSize);
            this.buffer.copy(newBuffer, 0, 0, this.offset);
            OptimizedBufferWriter.pool.release(this.buffer);
            this.buffer = newBuffer;
        }
    }

    /**
     * 写入无符号 8 位整数
     */
    public writeUInt8(value: number): this {
        this.ensureCapacity(1);
        this.buffer.writeUInt8(value, this.offset);
        this.offset += 1;
        return this;
    }

    /**
     * 写入无符号 16 位整数（Little-Endian）
     */
    public writeUInt16LE(value: number): this {
        this.ensureCapacity(2);
        this.buffer.writeUInt16LE(value, this.offset);
        this.offset += 2;
        return this;
    }

    /**
     * 写入无符号 32 位整数（Little-Endian）
     */
    public writeUInt32LE(value: number): this {
        this.ensureCapacity(4);
        this.buffer.writeUInt32LE(value, this.offset);
        this.offset += 4;
        return this;
    }

    /**
     * 写入有符号 32 位整数（Little-Endian）
     */
    public writeInt32LE(value: number): this {
        this.ensureCapacity(4);
        this.buffer.writeInt32LE(value, this.offset);
        this.offset += 4;
        return this;
    }

    /**
     * 写入无符号 64 位整数（Little-Endian）
     */
    public writeBigUInt64LE(value: bigint): this {
        this.ensureCapacity(8);
        this.buffer.writeBigUInt64LE(value, this.offset);
        this.offset += 8;
        return this;
    }

    /**
     * 写入有符号 64 位整数（Little-Endian）
     */
    public writeBigInt64LE(value: bigint): this {
        this.ensureCapacity(8);
        this.buffer.writeBigInt64LE(value, this.offset);
        this.offset += 8;
        return this;
    }

    /**
     * 写入 32 位浮点数（Little-Endian）
     */
    public writeFloatLE(value: number): this {
        this.ensureCapacity(4);
        this.buffer.writeFloatLE(value, this.offset);
        this.offset += 4;
        return this;
    }

    /**
     * 写入 64 位双精度浮点数（Little-Endian）
     */
    public writeDoubleLE(value: number): this {
        this.ensureCapacity(8);
        this.buffer.writeDoubleLE(value, this.offset);
        this.offset += 8;
        return this;
    }

    /**
     * 写入字符串（带长度前缀）
     */
    public writeString(value: string): this {
        const byteLength = Buffer.byteLength(value, "utf8");
        this.writeUInt32LE(byteLength);
        this.ensureCapacity(byteLength);
        this.buffer.write(value, this.offset, "utf8");
        this.offset += byteLength;
        return this;
    }

    /**
     * 写入字符串（不带长度前缀）
     */
    public writeStringRaw(value: string): this {
        const byteLength = Buffer.byteLength(value, "utf8");
        this.ensureCapacity(byteLength);
        this.buffer.write(value, this.offset, "utf8");
        this.offset += byteLength;
        return this;
    }

    /**
     * 写入 Buffer
     */
    public writeBuffer(value: Buffer): this {
        this.ensureCapacity(value.length);
        value.copy(this.buffer, this.offset);
        this.offset += value.length;
        return this;
    }

    /**
     * 写入带长度前缀的 Buffer
     */
    public writeBufferWithLength(value: Buffer): this {
        this.writeUInt32LE(value.length);
        this.writeBuffer(value);
        return this;
    }

    /**
     * 写入字节数组
     */
    public writeBytes(values: number[]): this {
        this.ensureCapacity(values.length);
        for (let i = 0; i < values.length; i++) {
            this.buffer[this.offset + i] = values[i]!;
        }
        this.offset += values.length;
        return this;
    }

    /**
     * 填充指定字节
     */
    public fill(value: number, count: number): this {
        this.ensureCapacity(count);
        this.buffer.fill(value, this.offset, this.offset + count);
        this.offset += count;
        return this;
    }

    /**
     * 跳过指定字节数
     */
    public skip(count: number): this {
        this.ensureCapacity(count);
        this.offset += count;
        return this;
    }

    /**
     * 在指定位置写入值
     */
    public writeAt(offset: number, value: number, size: 1 | 2 | 4 | 8): this {
        switch (size) {
            case 1:
                this.buffer.writeUInt8(value, offset);
                break;
            case 2:
                this.buffer.writeUInt16LE(value, offset);
                break;
            case 4:
                this.buffer.writeUInt32LE(value, offset);
                break;
            case 8:
                this.buffer.writeBigUInt64LE(BigInt(value), offset);
                break;
        }
        return this;
    }

    /**
     * 重置写入器
     */
    public reset(): this {
        this.offset = 0;
        return this;
    }

    /**
     * 释放资源
     */
    public dispose(): void {
        OptimizedBufferWriter.pool.release(this.buffer);
    }
}

/**
 * 优化的 Buffer 读取器
 */
export class OptimizedBufferReader {
    private buffer: Buffer;
    private offset: number;
    private startOffset: number;

    /**
     * 创建优化的 Buffer 读取器
     * @param buffer 要读取的 Buffer
     * @param startOffset 起始偏移量
     */
    constructor(buffer: Buffer, startOffset: number = 0) {
        this.buffer = buffer;
        this.startOffset = startOffset;
        this.offset = startOffset;
    }

    /**
     * 获取当前读取位置
     */
    public get position(): number {
        return this.offset;
    }

    /**
     * 设置读取位置
     */
    public set position(value: number) {
        this.offset = value;
    }

    /**
     * 获取剩余可读字节数
     */
    public get remaining(): number {
        return this.buffer.length - this.offset;
    }

    /**
     * 获取总长度
     */
    public get length(): number {
        return this.buffer.length;
    }

    /**
     * 检查是否有足够的可读数据
     */
    public hasRemaining(required: number): boolean {
        return this.remaining >= required;
    }

    /**
     * 读取无符号 8 位整数
     */
    public readUInt8(): number {
        const value = this.buffer.readUInt8(this.offset);
        this.offset += 1;
        return value;
    }

    /**
     * 读取无符号 16 位整数（Little-Endian）
     */
    public readUInt16LE(): number {
        const value = this.buffer.readUInt16LE(this.offset);
        this.offset += 2;
        return value;
    }

    /**
     * 读取无符号 32 位整数（Little-Endian）
     */
    public readUInt32LE(): number {
        const value = this.buffer.readUInt32LE(this.offset);
        this.offset += 4;
        return value;
    }

    /**
     * 读取有符号 32 位整数（Little-Endian）
     */
    public readInt32LE(): number {
        const value = this.buffer.readInt32LE(this.offset);
        this.offset += 4;
        return value;
    }

    /**
     * 读取无符号 64 位整数（Little-Endian）
     */
    public readBigUInt64LE(): bigint {
        const value = this.buffer.readBigUInt64LE(this.offset);
        this.offset += 8;
        return value;
    }

    /**
     * 读取有符号 64 位整数（Little-Endian）
     */
    public readBigInt64LE(): bigint {
        const value = this.buffer.readBigInt64LE(this.offset);
        this.offset += 8;
        return value;
    }

    /**
     * 读取 32 位浮点数（Little-Endian）
     */
    public readFloatLE(): number {
        const value = this.buffer.readFloatLE(this.offset);
        this.offset += 4;
        return value;
    }

    /**
     * 读取 64 位双精度浮点数（Little-Endian）
     */
    public readDoubleLE(): number {
        const value = this.buffer.readDoubleLE(this.offset);
        this.offset += 8;
        return value;
    }

    /**
     * 读取字符串（带长度前缀）
     */
    public readString(): string {
        const length = this.readUInt32LE();
        const value = this.buffer.toString("utf8", this.offset, this.offset + length);
        this.offset += length;
        return value;
    }

    /**
     * 读取指定长度的字符串
     */
    public readStringFixed(length: number): string {
        const value = this.buffer.toString("utf8", this.offset, this.offset + length);
        this.offset += length;
        return value;
    }

    /**
     * 读取指定长度的 Buffer
     */
    public readBuffer(length: number): Buffer {
        const value = this.buffer.subarray(this.offset, this.offset + length);
        this.offset += length;
        return value;
    }

    /**
     * 读取带长度前缀的 Buffer
     */
    public readBufferWithLength(): Buffer {
        const length = this.readUInt32LE();
        return this.readBuffer(length);
    }

    /**
     * 读取字节数组
     */
    public readBytes(length: number): number[] {
        const result: number[] = [];
        for (let i = 0; i < length; i++) {
            result.push(this.buffer[this.offset + i]!);
        }
        this.offset += length;
        return result;
    }

    /**
     * 跳过指定字节数
     */
    public skip(bytes: number): this {
        this.offset += bytes;
        return this;
    }

    /**
     * 回退指定字节数
     */
    public back(bytes: number): this {
        this.offset = Math.max(this.startOffset, this.offset - bytes);
        return this;
    }

    /**
     * 重置到起始位置
     */
    public reset(): this {
        this.offset = this.startOffset;
        return this;
    }

    /**
     * 查看当前位置的字节（不移动指针）
     */
    public peek(): number {
        return this.buffer[this.offset]!;
    }

    /**
     * 查看指定偏移量的值（不移动指针）
     */
    public peekAt(offset: number, size: 1 | 2 | 4 | 8): number | bigint {
        switch (size) {
            case 1:
                return this.buffer.readUInt8(offset);
            case 2:
                return this.buffer.readUInt16LE(offset);
            case 4:
                return this.buffer.readUInt32LE(offset);
            case 8:
                return this.buffer.readBigUInt64LE(offset);
        }
    }

    /**
     * 获取底层 Buffer
     */
    public getBuffer(): Buffer {
        return this.buffer;
    }
}

/**
 * 快速序列化工具
 * 提供常用数据结构的快速序列化方法
 */
export class FastSerializer {
    /**
     * 序列化键值对
     */
    public static serializeKeyValue(key: string, value: Buffer): Buffer {
        const keyLen = Buffer.byteLength(key, "utf8");
        const totalLen = 4 + keyLen + 4 + value.length;

        const writer = new OptimizedBufferWriter(totalLen);
        writer.writeUInt32LE(keyLen);
        writer.writeStringRaw(key);
        writer.writeUInt32LE(value.length);
        writer.writeBuffer(value);

        return writer.data;
    }

    /**
     * 反序列化键值对
     */
    public static deserializeKeyValue(
        buffer: Buffer,
        offset: number = 0,
    ): {
        key: string;
        value: Buffer;
        bytesRead: number;
    } {
        const reader = new OptimizedBufferReader(buffer, offset);

        const keyLen = reader.readUInt32LE();
        const key = reader.readStringFixed(keyLen);
        const valueLen = reader.readUInt32LE();
        const value = reader.readBuffer(valueLen);

        return {
            key,
            value,
            bytesRead: reader.position - offset,
        };
    }

    /**
     * 序列化字符串数组
     */
    public static serializeStringArray(strings: string[]): Buffer {
        let totalLen = 4;
        for (const s of strings) {
            totalLen += 4 + Buffer.byteLength(s, "utf8");
        }

        const writer = new OptimizedBufferWriter(totalLen);
        writer.writeUInt32LE(strings.length);

        for (const s of strings) {
            writer.writeString(s);
        }

        return writer.data;
    }

    /**
     * 反序列化字符串数组
     */
    public static deserializeStringArray(
        buffer: Buffer,
        offset: number = 0,
    ): {
        strings: string[];
        bytesRead: number;
    } {
        const reader = new OptimizedBufferReader(buffer, offset);
        const count = reader.readUInt32LE();
        const strings: string[] = [];

        for (let i = 0; i < count; i++) {
            strings.push(reader.readString());
        }

        return {
            strings,
            bytesRead: reader.position - offset,
        };
    }

    /**
     * 序列化数字数组
     */
    public static serializeNumberArray(numbers: number[]): Buffer {
        const totalLen = 4 + numbers.length * 8;
        const writer = new OptimizedBufferWriter(totalLen);

        writer.writeUInt32LE(numbers.length);
        for (const n of numbers) {
            writer.writeDoubleLE(n);
        }

        return writer.data;
    }

    /**
     * 反序列化数字数组
     */
    public static deserializeNumberArray(
        buffer: Buffer,
        offset: number = 0,
    ): {
        numbers: number[];
        bytesRead: number;
    } {
        const reader = new OptimizedBufferReader(buffer, offset);
        const count = reader.readUInt32LE();
        const numbers: number[] = [];

        for (let i = 0; i < count; i++) {
            numbers.push(reader.readDoubleLE());
        }

        return {
            numbers,
            bytesRead: reader.position - offset,
        };
    }

    /**
     * 序列化对象（简单 JSON 序列化的优化版本）
     */
    public static serializeObject(obj: Record<string, unknown>): Buffer {
        const json = JSON.stringify(obj);
        const jsonBuffer = Buffer.from(json, "utf8");

        const writer = new OptimizedBufferWriter(4 + jsonBuffer.length);
        writer.writeUInt32LE(jsonBuffer.length);
        writer.writeBuffer(jsonBuffer);

        return writer.data;
    }

    /**
     * 反序列化对象
     */
    public static deserializeObject(
        buffer: Buffer,
        offset: number = 0,
    ): {
        obj: Record<string, unknown>;
        bytesRead: number;
    } {
        const reader = new OptimizedBufferReader(buffer, offset);
        const len = reader.readUInt32LE();
        const jsonBuffer = reader.readBuffer(len);
        const json = jsonBuffer.toString("utf8");

        return {
            obj: JSON.parse(json) as Record<string, unknown>,
            bytesRead: reader.position - offset,
        };
    }
}

/**
 * 批量 Buffer 合并工具
 */
export class BufferConcatenator {
    private buffers: Buffer[];
    private totalLength: number;

    constructor() {
        this.buffers = [];
        this.totalLength = 0;
    }

    /**
     * 添加 Buffer
     */
    public append(buffer: Buffer): this {
        this.buffers.push(buffer);
        this.totalLength += buffer.length;
        return this;
    }

    /**
     * 添加多个 Buffer
     */
    public appendMany(buffers: Buffer[]): this {
        for (const buffer of buffers) {
            this.append(buffer);
        }
        return this;
    }

    /**
     * 获取总长度
     */
    public get length(): number {
        return this.totalLength;
    }

    /**
     * 获取 Buffer 数量
     */
    public get count(): number {
        return this.buffers.length;
    }

    /**
     * 合并所有 Buffer
     */
    public concat(): Buffer {
        if (this.buffers.length === 0) {
            return Buffer.alloc(0);
        }

        if (this.buffers.length === 1) {
            return this.buffers[0]!;
        }

        const result = Buffer.allocUnsafe(this.totalLength);
        let offset = 0;

        for (const buffer of this.buffers) {
            buffer.copy(result, offset);
            offset += buffer.length;
        }

        return result;
    }

    /**
     * 清空
     */
    public clear(): this {
        this.buffers = [];
        this.totalLength = 0;
        return this;
    }
}
