/**
 * LightDB 存储引擎常量定义
 * 包含文件格式、魔数、版本等核心常量
 */

/** 魔数：用于标识 .light 文件格式 */
export const LIGHT_MAGIC = 0x4c494748;

/** 魔数的字符串表示：'LIGH' */
export const LIGHT_MAGIC_STRING = "LIGH";

/** 当前文件格式版本号 */
export const LIGHT_VERSION = 1;

/** 默认页面大小：4KB */
export const DEFAULT_PAGE_SIZE = 4096;

/** 默认 WAL 文件大小阈值：16MB，超过此值触发检查点 */
export const DEFAULT_WAL_THRESHOLD = 16 * 1024 * 1024;

/** Header 固定长度：魔数(4) + 版本(4) + 页面大小(4) + 上次快照LSN(8) + 保留(16) = 36字节 */
export const HEADER_SIZE = 36;

/** WAL 记录头部长度：LSN(8) + 事务ID(8) + 操作类型(1) + Key长度(4) + Value长度(4) + 校验和(4) = 29字节 */
export const WAL_RECORD_HEADER_SIZE = 29;

/** SHM Header 大小：WAL文件大小(8) + 最后完整页LSN(8) + 记录数量(4) + 校验和(4) = 24字节 */
export const SHM_HEADER_SIZE = 24;

/** SHM 哈希表条目大小：页面ID(4) + LSN(8) + WAL偏移(8) = 20字节 */
export const SHM_ENTRY_SIZE = 20;

/** SHM 哈希表默认容量 */
export const SHM_DEFAULT_CAPACITY = 1024;

/** 页面头部大小：页面ID(4) + 记录数(2) + 空闲空间(2) + 校验和(4) = 12字节 */
export const PAGE_HEADER_SIZE = 12;

/** 操作类型枚举值 */
export enum OperationType {
    /** 插入操作 */
    Insert = 0x01,
    /** 更新操作 */
    Update = 0x02,
    /** 删除操作 */
    Delete = 0x03,
    /** 批量插入 */
    BatchInsert = 0x04,
    /** 检查点标记 */
    Checkpoint = 0x10,
}

/** 文件扩展名 */
export const FILE_EXTENSIONS = {
    /** 主数据文件 */
    LIGHT: ".light",
    /** 预写日志文件 */
    WAL: ".wal",
    /** 共享内存索引文件 */
    SHM: ".shm",
    /** 临时文件 */
    TEMP: ".tmp",
} as const;
