/**
 * 文件操作工具类
 * 提供文件系统操作的封装
 */

import { promises as fs } from "fs";
import * as path from "path";
import { FileOperationError } from "../errors";

/**
 * 文件操作工具类
 * 提供静态方法用于文件系统操作
 */
export class FileUtils {
    /**
     * 检查文件是否存在
     * @param filePath 文件路径
     */
    public static async exists(filePath: string): Promise<boolean> {
        try {
            await fs.access(filePath);
            return true;
        } catch {
            return false;
        }
    }

    /**
     * 确保目录存在，不存在则创建
     * @param dirPath 目录路径
     */
    public static async ensureDir(dirPath: string): Promise<void> {
        try {
            await fs.mkdir(dirPath, { recursive: true });
        } catch (error) {
            const err = error as NodeJS.ErrnoException;
            if (err.code !== "EEXIST") {
                throw new FileOperationError(`Failed to create directory: ${dirPath}`, dirPath, "mkdir");
            }
        }
    }

    /**
     * 读取文件内容
     * @param filePath 文件路径
     */
    public static async readFile(filePath: string): Promise<Buffer> {
        try {
            return await fs.readFile(filePath);
        } catch (error) {
            throw new FileOperationError(`Failed to read file: ${filePath}`, filePath, "read");
        }
    }

    /**
     * 写入文件内容
     * @param filePath 文件路径
     * @param data 文件内容
     */
    public static async writeFile(filePath: string, data: Buffer): Promise<void> {
        try {
            await fs.writeFile(filePath, data);
        } catch (error) {
            throw new FileOperationError(`Failed to write file: ${filePath}`, filePath, "write");
        }
    }

    /**
     * 追加写入文件
     * @param filePath 文件路径
     * @param data 要追加的数据
     */
    public static async appendFile(filePath: string, data: Buffer): Promise<void> {
        try {
            await fs.appendFile(filePath, data);
        } catch (error) {
            throw new FileOperationError(`Failed to append to file: ${filePath}`, filePath, "append");
        }
    }

    /**
     * 删除文件
     * @param filePath 文件路径
     */
    public static async deleteFile(filePath: string): Promise<void> {
        try {
            await fs.unlink(filePath);
        } catch (error) {
            const err = error as NodeJS.ErrnoException;
            if (err.code !== "ENOENT") {
                throw new FileOperationError(`Failed to delete file: ${filePath}`, filePath, "delete");
            }
        }
    }

    /**
     * 原子性重命名文件
     * @param oldPath 原文件路径
     * @param newPath 新文件路径
     */
    public static async rename(oldPath: string, newPath: string): Promise<void> {
        try {
            await fs.rename(oldPath, newPath);
        } catch (error) {
            throw new FileOperationError(`Failed to rename file from ${oldPath} to ${newPath}`, oldPath, "rename");
        }
    }

    /**
     * 获取文件大小
     * @param filePath 文件路径
     */
    public static async getFileSize(filePath: string): Promise<number> {
        try {
            const stats = await fs.stat(filePath);
            return stats.size;
        } catch (error) {
            throw new FileOperationError(`Failed to get file size: ${filePath}`, filePath, "stat");
        }
    }

    /**
     * 同步文件到磁盘
     * @param filePath 文件路径
     */
    public static async syncFile(filePath: string): Promise<void> {
        let handle: fs.FileHandle | null = null;
        try {
            handle = await fs.open(filePath, "r+");
            await handle.sync();
        } catch (error) {
            throw new FileOperationError(`Failed to sync file: ${filePath}`, filePath, "sync");
        } finally {
            if (handle) {
                await handle.close();
            }
        }
    }

    /**
     * 打开文件句柄
     * @param filePath 文件路径
     * @param flags 打开标志
     */
    public static async openFile(filePath: string, flags: string): Promise<fs.FileHandle> {
        try {
            return await fs.open(filePath, flags);
        } catch (error) {
            throw new FileOperationError(`Failed to open file: ${filePath}`, filePath, "open");
        }
    }

    /**
     * 从指定位置读取文件
     * @param handle 文件句柄
     * @param buffer 目标缓冲区
     * @param offset 缓冲区偏移
     * @param length 读取长度
     * @param position 文件位置
     */
    public static async readFromPosition(
        handle: fs.FileHandle,
        buffer: Buffer,
        offset: number,
        length: number,
        position: number,
    ): Promise<number> {
        const result = await handle.read(buffer, offset, length, position);
        return result.bytesRead;
    }

    /**
     * 在指定位置写入文件
     * @param handle 文件句柄
     * @param buffer 源缓冲区
     * @param offset 缓冲区偏移
     * @param length 写入长度
     * @param position 文件位置
     */
    public static async writeToPosition(
        handle: fs.FileHandle,
        buffer: Buffer,
        offset: number,
        length: number,
        position: number,
    ): Promise<number> {
        const result = await handle.write(buffer, offset, length, position);
        return result.bytesWritten;
    }

    /**
     * 截断文件到指定大小
     * @param filePath 文件路径
     * @param size 目标大小
     */
    public static async truncateFile(filePath: string, size: number): Promise<void> {
        try {
            await fs.truncate(filePath, size);
        } catch (error) {
            throw new FileOperationError(`Failed to truncate file: ${filePath}`, filePath, "truncate");
        }
    }

    /**
     * 构建完整文件路径
     * @param basePath 基础路径
     * @param name 文件名（不含扩展名）
     * @param extension 文件扩展名
     */
    public static buildFilePath(basePath: string, name: string, extension: string): string {
        return path.join(basePath, name + extension);
    }

    /**
     * 解析文件名（不含扩展名）
     * @param filePath 文件路径
     */
    public static getFileName(filePath: string): string {
        const basename = path.basename(filePath);
        const ext = path.extname(basename);
        return basename.slice(0, basename.length - ext.length);
    }

    /**
     * 获取目录路径
     * @param filePath 文件路径
     */
    public static getDirPath(filePath: string): string {
        return path.dirname(filePath);
    }
}

/**
 * 文件句柄包装类
 * 提供更高级的文件操作接口
 */
export class FileHandleWrapper {
    private handle: fs.FileHandle | null = null;
    private readonly filePath: string;
    private readonly flags: string;

    /**
     * 创建文件句柄包装
     * @param filePath 文件路径
     * @param flags 打开标志
     */
    constructor(filePath: string, flags: string) {
        this.filePath = filePath;
        this.flags = flags;
    }

    /**
     * 打开文件
     */
    public async open(): Promise<void> {
        this.handle = await FileUtils.openFile(this.filePath, this.flags);
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
     * 确保文件已打开
     */
    private ensureOpen(): void {
        if (!this.handle) {
            throw new FileOperationError("File handle is not open", this.filePath, "ensureOpen");
        }
    }

    /**
     * 读取数据
     * @param buffer 目标缓冲区
     * @param offset 缓冲区偏移
     * @param length 读取长度
     * @param position 文件位置
     */
    public async read(buffer: Buffer, offset: number, length: number, position: number): Promise<number> {
        this.ensureOpen();
        return FileUtils.readFromPosition(this.handle!, buffer, offset, length, position);
    }

    /**
     * 写入数据
     * @param buffer 源缓冲区
     * @param offset 缓冲区偏移
     * @param length 写入长度
     * @param position 文件位置
     */
    public async write(buffer: Buffer, offset: number, length: number, position: number): Promise<number> {
        this.ensureOpen();
        return FileUtils.writeToPosition(this.handle!, buffer, offset, length, position);
    }

    /**
     * 同步到磁盘
     */
    public async sync(): Promise<void> {
        this.ensureOpen();
        await this.handle!.sync();
    }

    /**
     * 获取文件大小
     */
    public async size(): Promise<number> {
        this.ensureOpen();
        const stats = await this.handle!.stat();
        return stats.size;
    }

    /**
     * 获取文件路径
     */
    public get path(): string {
        return this.filePath;
    }

    /**
     * 检查是否已打开
     */
    public get isOpen(): boolean {
        return this.handle !== null;
    }
}
