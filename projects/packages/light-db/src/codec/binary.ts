/**
 * 浏览器原生二进制编解码
 *
 * 公共类型只用 `Uint8Array`，不依赖 Node `Buffer`。
 */

import { crc32 } from "./checksum";

/** UTF-8 编码器 */
const TEXT_ENCODER = new TextEncoder();

/** UTF-8 解码器 */
const TEXT_DECODER = new TextDecoder("utf-8");

/**
 * 二进制辅助方法
 */
export class BinaryUtils {
    /**
     * 分配初始化为零的字节数组
     * @param size 大小
     */
    public static alloc(size: number): Uint8Array {
        return new Uint8Array(size);
    }

    /**
     * 拼接多个字节数组
     * @param parts 字节数组列表
     */
    public static concat(parts: readonly Uint8Array[]): Uint8Array {
        let total = 0;
        for (const part of parts) {
            total += part.length;
        }

        const result = new Uint8Array(total);
        let offset = 0;
        for (const part of parts) {
            result.set(part, offset);
            offset += part.length;
        }
        return result;
    }

    /**
     * 计算字符串的 UTF-8 字节长度
     * @param value 字符串
     */
    public static byteLength(value: string): number {
        return TEXT_ENCODER.encode(value).length;
    }

    /**
     * 编码字符串为 UTF-8 字节
     * @param value 字符串
     */
    public static encode(value: string): Uint8Array {
        return TEXT_ENCODER.encode(value);
    }

    /**
     * 解码 UTF-8 字节为字符串
     * @param data 字节
     * @param offset 起始偏移量
     * @param length 长度
     */
    public static decode(data: Uint8Array, offset: number = 0, length?: number): string {
        const end = length === undefined ? data.length : offset + length;
        return TEXT_DECODER.decode(data.subarray(offset, end));
    }

    /**
     * 判断两个字节数组内容是否相同
     * @param a 第一个数组
     * @param b 第二个数组
     */
    public static equals(a: Uint8Array, b: Uint8Array): boolean {
        if (a.length !== b.length) {
            return false;
        }
        for (let i = 0; i < a.length; i++) {
            if (a[i] !== b[i]) {
                return false;
            }
        }
        return true;
    }

    /**
     * 复制字节数组
     * @param source 源数组
     */
    public static copy(source: Uint8Array): Uint8Array {
        return new Uint8Array(source);
    }
}

/**
 * 顺序字节写入器
 *
 * 容量不足时自动扩展，写入 `data` 时返回已写区间视图。
 */
export class ByteWriter {
    private buffer: Uint8Array;
    private view: DataView;
    private offset: number;

    /**
     * 创建写入器
     * @param size 初始容量
     */
    constructor(size: number = 64) {
        this.buffer = new Uint8Array(Math.max(1, size));
        this.view = new DataView(this.buffer.buffer);
        this.offset = 0;
    }

    /** 当前写入位置 */
    public get position(): number {
        return this.offset;
    }

    /** 已写入的数据视图 */
    public get data(): Uint8Array {
        return this.buffer.subarray(0, this.offset);
    }

    /**
     * 确保容量
     * @param additional 需要的额外字节数
     */
    private ensureCapacity(additional: number): void {
        const required = this.offset + additional;
        if (required <= this.buffer.length) {
            return;
        }

        let capacity = this.buffer.length * 2;
        while (capacity < required) {
            capacity *= 2;
        }

        const next = new Uint8Array(capacity);
        next.set(this.buffer.subarray(0, this.offset), 0);
        this.buffer = next;
        this.view = new DataView(next.buffer);
    }

    /**
     * 写入 8 位无符号整数
     * @param value 值
     */
    public writeUInt8(value: number): this {
        this.ensureCapacity(1);
        this.buffer[this.offset++] = value & 0xff;
        return this;
    }

    /**
     * 写入 16 位无符号整数（小端）
     * @param value 值
     */
    public writeUInt16LE(value: number): this {
        this.ensureCapacity(2);
        this.view.setUint16(this.offset, value, true);
        this.offset += 2;
        return this;
    }

    /**
     * 写入 32 位无符号整数（小端）
     * @param value 值
     */
    public writeUInt32LE(value: number): this {
        this.ensureCapacity(4);
        this.view.setUint32(this.offset, value >>> 0, true);
        this.offset += 4;
        return this;
    }

    /**
     * 写入 64 位无符号整数（小端）
     * @param value 值
     */
    public writeBigUInt64LE(value: bigint): this {
        this.ensureCapacity(8);
        this.view.setBigUint64(this.offset, value, true);
        this.offset += 8;
        return this;
    }

    /**
     * 写入原始字节
     * @param value 字节
     */
    public writeBytes(value: Uint8Array): this {
        this.ensureCapacity(value.length);
        this.buffer.set(value, this.offset);
        this.offset += value.length;
        return this;
    }

    /**
     * 写入带 32 位长度前缀的字节
     * @param value 字节
     */
    public writeBytesWithLength(value: Uint8Array): this {
        this.writeUInt32LE(value.length);
        return this.writeBytes(value);
    }

    /**
     * 写入不带长度前缀的 UTF-8 字符串
     * @param value 字符串
     */
    public writeStringRaw(value: string): this {
        return this.writeBytes(BinaryUtils.encode(value));
    }

    /**
     * 写入带 32 位长度前缀的 UTF-8 字符串
     * @param value 字符串
     */
    public writeString(value: string): this {
        return this.writeBytesWithLength(BinaryUtils.encode(value));
    }

    /**
     * 写入从 `startOffset` 到当前位置的 CRC32 校验和
     * @param startOffset 校验范围起点
     */
    public writeChecksum(startOffset: number = 0): this {
        const checksum = crc32(this.buffer, startOffset, this.offset - startOffset);
        return this.writeUInt32LE(checksum);
    }
}

/**
 * 顺序字节读取器
 */
export class ByteReader {
    private readonly buffer: Uint8Array;
    private readonly view: DataView;
    private offset: number;

    /**
     * 创建读取器
     * @param buffer 数据
     * @param startOffset 起始偏移量
     */
    constructor(buffer: Uint8Array, startOffset: number = 0) {
        this.buffer = buffer;
        this.view = new DataView(buffer.buffer, buffer.byteOffset, buffer.byteLength);
        this.offset = startOffset;
    }

    /** 当前读取位置 */
    public get position(): number {
        return this.offset;
    }

    /** 设置读取位置 */
    public set position(value: number) {
        this.offset = value;
    }

    /** 剩余可读字节数 */
    public get remaining(): number {
        return this.buffer.length - this.offset;
    }

    /**
     * 是否有足够的可读数据
     * @param required 需要的字节数
     */
    public hasRemaining(required: number): boolean {
        return this.remaining >= required;
    }

    /**
     * 读取 8 位无符号整数
     */
    public readUInt8(): number {
        return this.buffer[this.offset++]!;
    }

    /**
     * 读取 16 位无符号整数（小端）
     */
    public readUInt16LE(): number {
        const value = this.view.getUint16(this.offset, true);
        this.offset += 2;
        return value;
    }

    /**
     * 读取 32 位无符号整数（小端）
     */
    public readUInt32LE(): number {
        const value = this.view.getUint32(this.offset, true);
        this.offset += 4;
        return value;
    }

    /**
     * 读取 64 位无符号整数（小端）
     */
    public readBigUInt64LE(): bigint {
        const value = this.view.getBigUint64(this.offset, true);
        this.offset += 8;
        return value;
    }

    /**
     * 读取指定长度的字节视图
     * @param length 长度
     */
    public readBytes(length: number): Uint8Array {
        const value = this.buffer.subarray(this.offset, this.offset + length);
        this.offset += length;
        return value;
    }

    /**
     * 读取带 32 位长度前缀的字节
     */
    public readBytesWithLength(): Uint8Array {
        return this.readBytes(this.readUInt32LE());
    }

    /**
     * 读取带 32 位长度前缀的 UTF-8 字符串
     */
    public readString(): string {
        return BinaryUtils.decode(this.readBytesWithLength());
    }

    /**
     * 读取 32 位校验和并校验 `[startOffset, 校验和之前)` 的区间
     * @param startOffset 校验范围起点
     */
    public readAndVerifyChecksum(startOffset: number): number {
        const expected = this.readUInt32LE();
        const actual = crc32(this.buffer, startOffset, this.offset - startOffset - 4);
        if (actual !== expected) {
            throw new ChecksumMismatchError(expected, actual, startOffset);
        }
        return expected;
    }

    /**
     * 跳过指定字节数
     * @param bytes 字节数
     */
    public skip(bytes: number): this {
        this.offset += bytes;
        return this;
    }
}

/**
 * 校验和不匹配
 *
 * codec 层不依赖具体错误模型，由上层转换成稳定错误码。
 */
export class ChecksumMismatchError extends Error {
    /** 期望的校验和 */
    public readonly expected: number;

    /** 实际计算出的校验和 */
    public readonly actual: number;

    /** 校验区间起点 */
    public readonly position: number;

    constructor(expected: number, actual: number, position: number) {
        super(`Checksum mismatch: expected ${expected}, got ${actual} at offset ${position}`);
        this.name = "ChecksumMismatchError";
        this.expected = expected;
        this.actual = actual;
        this.position = position;
        Object.setPrototypeOf(this, ChecksumMismatchError.prototype);
    }
}
