/**
 * LightDB 集合层类型定义
 * 包含所有集合相关的接口、类型和结构体定义
 */

/**
 * 主键类型
 * 支持字符串和数值类型
 */
export type PrimaryKeyType = string | number;

/**
 * 文档类型
 * 泛型 T 表示用户定义的文档结构
 */
export type Document<T> = T & Record<string, unknown>;

/**
 * 比较操作符条件
 */
export interface ComparisonOperators<T> {
    /** 等于 */
    $eq?: T;
    /** 不等于 */
    $ne?: T;
    /** 大于 */
    $gt?: T;
    /** 大于等于 */
    $gte?: T;
    /** 小于 */
    $lt?: T;
    /** 小于等于 */
    $lte?: T;
}

/**
 * 数组操作符条件
 */
export interface ArrayOperators<T> {
    /** 包含于数组 */
    $in?: T[];
    /** 不包含于数组 */
    $nin?: T[];
}

/**
 * 存在性操作符条件
 */
export interface ExistenceOperators {
    /** 字段是否存在 */
    $exists?: boolean;
}

/**
 * 正则表达式操作符条件
 */
export interface RegexOperators {
    /** 正则表达式匹配 */
    $regex?: RegExp | string;
    /** 正则表达式选项 */
    $options?: string;
}

/**
 * 字段查询条件
 * 可以是精确值、比较操作符或组合操作符
 */
export type FieldCondition<T> = T | ComparisonOperators<T> | ArrayOperators<T> | ExistenceOperators | RegexOperators;

/**
 * 逻辑操作符条件
 */
export interface LogicalOperators<T> {
    /** 逻辑与 */
    $and?: QueryFilter<T>[];
    /** 逻辑或 */
    $or?: QueryFilter<T>[];
    /** 逻辑非 */
    $not?: QueryFilter<T>;
    /** 逻辑非（等同于 $not） */
    $nor?: QueryFilter<T>[];
}

/**
 * 查询过滤器
 * 用于筛选文档
 */
export type QueryFilter<T> = {
    [K in keyof T]?: FieldCondition<T[K]>;
} & LogicalOperators<T>;

/**
 * 更新操作符
 */
export interface UpdateOperators<T> {
    /** 设置字段值 */
    $set?: Partial<T>;
    /** 删除字段 */
    $unset?: Partial<Record<keyof T, true>>;
    /** 字段值增加 */
    $inc?: Partial<Record<keyof T, number>>;
    /** 字段值乘法 */
    $mul?: Partial<Record<keyof T, number>>;
    /** 字段值重命名 */
    $rename?: Partial<Record<keyof T, string>>;
    /** 设置字段值为当前日期 */
    $currentDate?: Partial<Record<keyof T, true | { $type: "date" | "timestamp" }>>;
    /** 数组添加元素 */
    $push?: Partial<Record<keyof T, unknown | { $each?: unknown[]; $position?: number; $slice?: number }>>;
    /** 数组删除元素 */
    $pull?: Partial<Record<keyof T, unknown | QueryFilter<unknown>>>;
    /** 数组添加元素（去重） */
    $addToSet?: Partial<Record<keyof T, unknown | { $each?: unknown[] }>>;
    /** 数组弹出元素 */
    $pop?: Partial<Record<keyof T, 1 | -1>>;
}

/**
 * 更新操作
 * 可以是部分文档或更新操作符
 */
export type UpdateOperation<T> = Partial<T> | UpdateOperators<T>;

/**
 * 排序方向
 */
export type SortDirection = 1 | -1 | "asc" | "desc";

/**
 * 排序条件
 */
export type SortCondition<T> = Partial<Record<keyof T, SortDirection>>;

/**
 * 查询选项
 */
export interface QueryOptions<T> {
    /** 查询过滤器 */
    where?: QueryFilter<T>;
    /** 排序条件 */
    sort?: SortCondition<T>;
    /** 跳过记录数 */
    offset?: number;
    /** 返回记录数限制 */
    limit?: number;
    /** 返回的字段（投影） */
    fields?: Array<keyof T>;
}

/**
 * 插入选项
 */
export interface InsertOptions {
    /** 如果主键已存在是否抛出异常 */
    throwOnDuplicate?: boolean;
}

/**
 * 更新选项
 */
export interface UpdateOptions {
    /** 如果记录不存在是否抛出异常 */
    throwOnMissing?: boolean;
    /** 是否更新多条记录 */
    multi?: boolean;
    /** 如果记录不存在是否插入 */
    upsert?: boolean;
}

/**
 * 删除选项
 */
export interface DeleteOptions {
    /** 如果记录不存在是否抛出异常 */
    throwOnMissing?: boolean;
    /** 是否删除多条记录 */
    multi?: boolean;
}

/**
 * 插入结果
 */
export interface InsertResult<T> {
    /** 插入是否成功 */
    success: boolean;
    /** 插入的文档数量 */
    insertedCount: number;
    /** 插入的文档 */
    insertedDocs: T[];
}

/**
 * 批量插入结果
 */
export interface InsertManyResult<T> {
    /** 插入是否成功 */
    success: boolean;
    /** 成功插入的文档数量 */
    insertedCount: number;
    /** 失败的文档数量 */
    failedCount: number;
    /** 插入的文档 */
    insertedDocs: T[];
    /** 错误列表 */
    errors?: Array<{ index: number; error: string }>;
}

/**
 * 更新结果
 */
export interface UpdateResult {
    /** 更新是否成功 */
    success: boolean;
    /** 匹配的文档数量 */
    matchedCount: number;
    /** 修改的文档数量 */
    modifiedCount: number;
    /** 是否插入了新文档（upsert） */
    upserted?: boolean;
    /** 插入的文档主键（upsert） */
    upsertedId?: PrimaryKeyType;
}

/**
 * 删除结果
 */
export interface DeleteResult {
    /** 删除是否成功 */
    success: boolean;
    /** 删除的文档数量 */
    deletedCount: number;
}

/**
 * 查询结果
 */
export interface FindResult<T> {
    /** 查询到的文档 */
    docs: T[];
    /** 总文档数（不考虑分页） */
    total?: number;
    /** 是否使用了索引 */
    usedIndex?: boolean;
    /** 使用的索引名称 */
    indexName?: string;
}

/**
 * 二级索引配置
 */
export interface IndexConfig<T = unknown> {
    /** 索引名称 */
    name: string;
    /** 索引字段名 */
    fieldName: keyof T & string;
    /** 是否唯一索引 */
    unique?: boolean;
    /** 是否稀疏索引（不索引 null/undefined 值） */
    sparse?: boolean;
}

/**
 * 集合配置选项
 */
export interface CollectionOptions<T = unknown> {
    /** 主键字段名 */
    primaryKey: keyof T & string;
    /** 二级索引配置 */
    indexes?: IndexConfig<T>[];
    /** 集合名称 */
    name?: string;
}

/**
 * 集合统计信息
 */
export interface CollectionStats {
    /** 集合名称 */
    name: string;
    /** 文档数量 */
    documentCount: number;
    /** 索引数量 */
    indexCount: number;
    /** 索引统计信息 */
    indexes: Array<{
        name: string;
        type: "primary" | "secondary";
        fieldName: string;
        unique: boolean;
        keyCount: number;
    }>;
    /** 估算的内存使用量（字节） */
    memoryUsage: number;
}

/**
 * 集合事件类型
 */
export type CollectionEventType = "insert" | "update" | "delete";

/**
 * 集合事件监听器
 */
export type CollectionEventListener<T> = (
    event: CollectionEventType,
    data: {
        doc?: T;
        oldDoc?: T;
        docs?: T[];
        filter?: QueryFilter<T>;
    },
) => void;

/**
 * 查询游标选项
 */
export interface CursorOptions {
    /** 批量获取大小 */
    batchSize?: number;
    /** 超时时间（毫秒） */
    timeout?: number;
}

/**
 * 聚合管道阶段
 */
export type AggregationStage<T> =
    | { $match: QueryFilter<T> }
    | {
          $group: { _id: string | Record<string, unknown>; [key: string]: unknown };
      }
    | { $sort: SortCondition<T> }
    | { $limit: number }
    | { $skip: number }
    | { $project: Partial<Record<keyof T, 0 | 1 | Record<string, unknown>>> }
    | { $unwind: string | { path: string; preserveNullAndEmptyArrays?: boolean } }
    | {
          $lookup: {
              from: string;
              localField: string;
              foreignField: string;
              as: string;
          };
      };
