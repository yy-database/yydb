/**
 * LightDB 集合层
 *
 * 提供类型安全的 CRUD 操作和链式查询 API
 *
 * @packageDocumentation
 */

// 类型导出
export {
    PrimaryKeyType,
    Document,
    ComparisonOperators,
    ArrayOperators,
    ExistenceOperators,
    RegexOperators,
    FieldCondition,
    LogicalOperators,
    QueryFilter,
    UpdateOperators,
    UpdateOperation,
    SortDirection,
    SortCondition,
    QueryOptions,
    InsertOptions,
    UpdateOptions,
    DeleteOptions,
    InsertResult,
    InsertManyResult,
    UpdateResult,
    DeleteResult,
    FindResult,
    IndexConfig,
    CollectionOptions,
    CollectionStats,
    CollectionEventType,
    CollectionEventListener,
    CursorOptions,
    AggregationStage,
} from "./types";

// 查询解析器导出
export { QueryParser, QueryParseResult } from "./QueryParser";

// 查询构建器导出
export { QueryBuilder, QueryExecutor } from "./QueryBuilder";

// 集合类导出
export { Collection, CollectionError } from "./Collection";
