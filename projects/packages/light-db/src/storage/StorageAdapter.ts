/**
 * 存储适配器合同
 *
 * 这是 `@yydb/light-db` 唯一的平台边界。上层只认识逻辑文件名与随机访问句柄，
 * 不认识本地路径、OPFS 目录结构或任何 Node 文件 API。
 */

/** 逻辑文件名，对外不暴露扩展名与目录结构 */
export type StorageFile = "main" | "main.tmp" | "wal" | "shm";

/** 所有逻辑文件 */
export const STORAGE_FILES: readonly StorageFile[] = ["main", "main.tmp", "wal", "shm"];

/** 打开模式 */
export type StorageOpenMode = "read" | "readwrite";

/** 适配器类型 */
export type StorageAdapterKind = "opfs" | "indexeddb" | "memory";

/**
 * 随机访问句柄
 *
 * 语义参照 OPFS `FileSystemSyncAccessHandle`：按位置读写、可截断、可 flush，
 * 不提供路径、目录或文件重命名能力。
 */
export interface StorageHandle {
    /**
     * 读取指定区间
     * @param position 起始位置
     * @param length 期望长度，超出文件末尾时返回实际可读部分
     */
    read(position: number, length: number): Promise<Uint8Array>;

    /**
     * 写入指定位置，必要时扩展文件
     * @param position 起始位置
     * @param data 数据
     */
    write(position: number, data: Uint8Array): Promise<void>;

    /**
     * 截断到指定大小
     * @param size 目标大小
     */
    truncate(size: number): Promise<void>;

    /** 将已写入的数据持久化到宿主 */
    sync(): Promise<void>;

    /** 当前文件大小 */
    size(): Promise<number>;

    /** 关闭句柄 */
    close(): Promise<void>;
}

/**
 * 单个数据库的存储适配器
 *
 * 适配器只管理四个逻辑文件，原子发布由 `rename` 完成。
 */
export interface StorageAdapter {
    /** 适配器类型 */
    readonly kind: StorageAdapterKind;

    /** 数据是否在会话结束后仍然存在 */
    readonly persistent: boolean;

    /**
     * 打开逻辑文件
     * @param file 逻辑文件名
     * @param mode 打开模式，默认 `readwrite`
     */
    open(file: StorageFile, mode?: StorageOpenMode): Promise<StorageHandle>;

    /**
     * 判断逻辑文件是否存在
     * @param file 逻辑文件名
     */
    exists(file: StorageFile): Promise<boolean>;

    /**
     * 删除逻辑文件，不存在时视为成功
     * @param file 逻辑文件名
     */
    delete(file: StorageFile): Promise<void>;

    /**
     * 原子重命名，用于发布快照
     * @param from 源文件
     * @param to 目标文件
     */
    rename(from: StorageFile, to: StorageFile): Promise<void>;

    /** 列出已存在的逻辑文件 */
    list(): Promise<StorageFile[]>;

    /** 关闭适配器 */
    close(): Promise<void>;
}

/**
 * 按数据库名打开适配器的工厂
 */
export interface StorageAdapterFactory {
    /** 适配器类型 */
    readonly kind: StorageAdapterKind;

    /** 数据是否在会话结束后仍然存在 */
    readonly persistent: boolean;

    /**
     * 打开指定数据库的适配器
     * @param name 数据库名（浏览器 storage namespace）
     */
    openDatabase(name: string): Promise<StorageAdapter>;
}
