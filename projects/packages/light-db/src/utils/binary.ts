/**
 * 二进制序列化工具类
 * 提供 Buffer 读写、校验和计算等核心功能
 */

import { ChecksumError } from "../errors";

/**
 * CRC32 查找表
 * 用于快速计算校验和
 */
const CRC32_TABLE: Uint32Array = new Uint32Array(256);

/** 初始化 CRC32 查找表 */
function initCRC32Table(): void {
    for (let i = 0; i < 256; i++) {
        let crc = i;
        for (let j = 0; j < 8; j++) {
            crc = crc & 1 ? 0xedb88320 ^ (crc >>> 1) : crc >>> 1;
        }
        CRC32_TABLE[i] = crc;
    }
}

initCRC32Table();

/**
 * 二进制工具类
 * 提供静态方法用于 Buffer 操作和校验和计算
 */
export class BinaryUtils {
    /**
     * 计算 Buffer 的 CRC32 校验和
     * @param data 要计算校验和的数据
     * @param offset 起始偏移量，默认 0
     * @param length 数据长度，默认到末尾
     * @returns CRC32 校验和（无符号 32 位整数）
     */
    public static calculateChecksum(data: Buffer, offset: number = 0, length?: number): number {
        const len = length ?? data.length - offset;
        let crc = 0xffffffff;

        for (let i = offset; i < offset + len; i++) {
            const tableIndex = (crc ^ data[i]!) & 0xff;
            const tableValue = CRC32_TABLE[tableIndex]!;
            crc = tableValue ^ (crc >>> 8);
        }

        return (crc ^ 0xffffffff) >>> 0;
    }

    /**
     * 验证校验和
     * @param data 数据内容
     * @param expectedChecksum 期望的校验和
     * @param offset 数据起始偏移量
     * @param length 数据长度
     * @throws ChecksumError 如果校验和不匹配
     */
    public static verifyChecksum(data: Buffer, expectedChecksum: number, offset: number = 0, length?: number): void {
        const actual = BinaryUtils.calculateChecksum(data, offset, length);
        if (actual !== expectedChecksum) {
            throw new ChecksumError(
                `Checksum verification failed: expected ${expectedChecksum}, got ${actual}`,
                expectedChecksum,
                actual,
                offset,
            );
        }
    }

    /**
     * 读取无符号 8 位整数
     * @param buffer 数据缓冲区
     * @param offset 偏移量
     */
    public static readUInt8(buffer: Buffer, offset: number): number {
        return buffer.readUInt8(offset);
    }

    /**
     * 写入无符号 8 位整数
     * @param buffer 数据缓冲区
     * @param value 值
     * @param offset 偏移量
     */
    public static writeUInt8(buffer: Buffer, value: number, offset: number): void {
        buffer.writeUInt8(value, offset);
    }

    /**
     * 读取无符号 16 位整数（Little-Endian）
     * @param buffer 数据缓冲区
     * @param offset 偏移量
     */
    public static readUInt16LE(buffer: Buffer, offset: number): number {
        return buffer.readUInt16LE(offset);
    }

    /**
     * 写入无符号 16 位整数（Little-Endian）
     * @param buffer 数据缓冲区
     * @param value 值
     * @param offset 偏移量
     */
    public static writeUInt16LE(buffer: Buffer, value: number, offset: number): void {
        buffer.writeUInt16LE(value, offset);
    }

    /**
     * 读取无符号 32 位整数（Little-Endian）
     * @param buffer 数据缓冲区
     * @param offset 偏移量
     */
    public static readUInt32LE(buffer: Buffer, offset: number): number {
        return buffer.readUInt32LE(offset);
    }

    /**
     * 写入无符号 32 位整数（Little-Endian）
     * @param buffer 数据缓冲区
     * @param value 值
     * @param offset 偏移量
     */
    public static writeUInt32LE(buffer: Buffer, value: number, offset: number): void {
        buffer.writeUInt32LE(value, offset);
    }

    /**
     * 读取有符号 32 位整数（Little-Endian）
     * @param buffer 数据缓冲区
     * @param offset 偏移量
     */
    public static readInt32LE(buffer: Buffer, offset: number): number {
        return buffer.readInt32LE(offset);
    }

    /**
     * 写入有符号 32 位整数（Little-Endian）
     * @param buffer 数据缓冲区
     * @param value 值
     * @param offset 偏移量
     */
    public static writeInt32LE(buffer: Buffer, value: number, offset: number): void {
        buffer.writeInt32LE(value, offset);
    }

    /**
     * 读取无符号 64 位整数（Little-Endian）
     * 返回 BigInt 类型
     * @param buffer 数据缓冲区
     * @param offset 偏移量
     */
    public static readBigUInt64LE(buffer: Buffer, offset: number): bigint {
        return buffer.readBigUInt64LE(offset);
    }

    /**
     * 写入无符号 64 位整数（Little-Endian）
     * @param buffer 数据缓冲区
     * @param value 值
     * @param offset 偏移量
     */
    public static writeBigUInt64LE(buffer: Buffer, value: bigint, offset: number): void {
        buffer.writeBigUInt64LE(value, offset);
    }

    /**
     * 读取字符串（UTF-8 编码）
     * @param buffer 数据缓冲区
     * @param offset 起始偏移量
     * @param length 字符串字节长度
     */
    public static readString(buffer: Buffer, offset: number, length: number): string {
        return buffer.toString("utf8", offset, offset + length);
    }

    /**
     * 写入字符串（UTF-8 编码）
     * @param buffer 数据缓冲区
     * @param value 字符串值
     * @param offset 起始偏移量
     * @returns 实际写入的字节数
     */
    public static writeString(buffer: Buffer, value: string, offset: number): number {
        return buffer.write(value, offset, "utf8");
    }

    /**
     * 计算字符串的字节长度（UTF-8 编码）
     * @param value 字符串值
     */
    public static stringByteLength(value: string): number {
        return Buffer.byteLength(value, "utf8");
    }

    /**
     * 分配指定大小的 Buffer（未初始化）
     * 比 Buffer.alloc 更快，但内容未定义
     * @param size Buffer 大小
     */
    public static allocUnsafe(size: number): Buffer {
        return Buffer.allocUnsafe(size);
    }

    /**
     * 分配指定大小的 Buffer（初始化为零）
     * @param size Buffer 大小
     */
    public static alloc(size: number): Buffer {
        return Buffer.alloc(size);
    }

    /**
     * 连接多个 Buffer
     * @param buffers Buffer 数组
     */
    public static concat(buffers: Buffer[]): Buffer {
        return Buffer.concat(buffers);
    }

    /**
     * 复制 Buffer 的一部分
     * @param source 源 Buffer
     * @param start 起始位置
     * @param end 结束位置
     */
    public static slice(source: Buffer, start: number, end?: number): Buffer {
        return source.subarray(start, end);
    }

    /**
     * 比较两个 Buffer 是否相等
     * @param a 第一个 Buffer
     * @param b 第二个 Buffer
     */
    public static equals(a: Buffer, b: Buffer): boolean {
        if (a.length !== b.length) {
            return false;
        }
        return a.equals(b);
    }
}

/**
 * Buffer 写入器
 * 提供便捷的顺序写入功能
 */
export class BufferWriter {
    private buffer: Buffer;
    private offset: number;

    /**
     * 创建 Buffer 写入器
     * @param size 初始 Buffer 大小
     */
    constructor(size: number) {
        this.buffer = Buffer.allocUnsafe(size);
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
     * 确保有足够的空间
     * @param additional 需要的额外空间
     */
    private ensureCapacity(additional: number): void {
        const required = this.offset + additional;
        if (required > this.buffer.length) {
            const newBuffer = Buffer.allocUnsafe(Math.max(required, this.buffer.length * 2));
            this.buffer.copy(newBuffer, 0, 0, this.offset);
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
     * 写入无符号 64 位整数（Little-Endian）
     */
    public writeBigUInt64LE(value: bigint): this {
        this.ensureCapacity(8);
        this.buffer.writeBigUInt64LE(value, this.offset);
        this.offset += 8;
        return this;
    }

    /**
     * 写入字符串（带长度前缀）
     */
    public writeString(value: string): this {
        const byteLength = BinaryUtils.stringByteLength(value);
        this.writeUInt32LE(byteLength);
        this.ensureCapacity(byteLength);
        BinaryUtils.writeString(this.buffer, value, this.offset);
        this.offset += byteLength;
        return this;
    }

    /**
     * 写入字符串（不带长度前缀）
     */
    public writeStringRaw(value: string): this {
        const byteLength = BinaryUtils.stringByteLength(value);
        this.ensureCapacity(byteLength);
        BinaryUtils.writeString(this.buffer, value, this.offset);
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
     * 写入校验和（计算从起始位置到当前位置的数据）
     */
    public writeChecksum(startOffset: number = 0): this {
        const checksum = BinaryUtils.calculateChecksum(this.buffer, startOffset, this.offset - startOffset);
        this.writeUInt32LE(checksum);
        return this;
    }
}

/**
 * Buffer 读取器
 * 提供便捷的顺序读取功能
 */
export class BufferReader {
    private buffer: Buffer;
    private offset: number;

    /**
     * 创建 Buffer 读取器
     * @param buffer 要读取的 Buffer
     * @param startOffset 起始偏移量，默认 0
     */
    constructor(buffer: Buffer, startOffset: number = 0) {
        this.buffer = buffer;
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
     * 读取无符号 64 位整数（Little-Endian）
     */
    public readBigUInt64LE(): bigint {
        const value = this.buffer.readBigUInt64LE(this.offset);
        this.offset += 8;
        return value;
    }

    /**
     * 读取字符串（带长度前缀）
     */
    public readString(): string {
        const length = this.readUInt32LE();
        const value = BinaryUtils.readString(this.buffer, this.offset, length);
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
     * 读取并验证校验和
     */
    public readAndVerifyChecksum(startOffset: number): number {
        const checksum = this.readUInt32LE();
        BinaryUtils.verifyChecksum(this.buffer, checksum, startOffset, this.offset - startOffset - 4);
        return checksum;
    }

    /**
     * 跳过指定字节数
     */
    public skip(bytes: number): this {
        this.offset += bytes;
        return this;
    }
}
