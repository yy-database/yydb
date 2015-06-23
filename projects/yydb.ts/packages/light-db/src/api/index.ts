/**
 * api 层导出
 *
 * 唯一的生产路径：{@link Database}（WASM-backed，委托给
 * `@yydb/yydb-unknown-wasm32` 的 Rust 语义核心）。
 */

export { Database, Statement, TransactionHandle, openLightDatabase } from './wasm-database';

export type {
    BatchChange,
    DateScalar,
    DecimalScalar,
    DoctorReport,
    FileManifestReference,
    FileScalar,
    LightDatabase,
    MigrateOptions,
    OpenOptions,
    ParameterDescriptor,
    QueryKind,
    ScalarValue,
    ScanOptions,
    ScanRow,
    SchemaFieldDescriptor,
    SchemaFieldType,
    SchemaIntrospection,
    SchemaTableDescriptor,
    SchemaVersion,
    SlotType,
    StatementInfo,
    TimeScalar,
    TimestampScalar,
    UniqueConstraintDescriptor,
} from './types';
