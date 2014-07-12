/**
 * `@yydb/yydb` 对 `@voml/vos` 的 re-export 门。
 *
 * 普通 YYDB 用户只依赖 `@yydb/yydb`，不必再显式安装或引入 `@voml/vos`。
 * 这里只转发 YYDB 作者面需要的 VOS 建模面：`v` builder、descriptor 类型与 `Infer` 推导。
 *
 * 刻意不转发：
 * - VOS source 发射：`@voml/vos` 不生成 `.vos` 文本（descriptor 直接编译 canonical artifact），
 *   源码书写与格式化属于 `@voml/vos-language`。
 * - `checkSource` / `VosDiagnostic`：VOS source 检查与语言层诊断属于 `@voml/vos-language`，
 *   不属于 `@voml/vos` 建模面，也不进 YYDB 作者面。
 *
 * 依赖边界见内部决策文档「VOS + YYDB TypeScript 门面设计」§1.1。
 */

export { v } from '@voml/vos';

export type { VBuiltinName, VItemName, VWrapperName } from '@voml/vos';

export type {
    AnyTypeDescriptor,
    Attribute,
    BuiltinKind,
    FieldDescriptor,
    Infer,
    RefDescriptor,
    ReferenceTarget,
    ScalarKind,
    TypeDescriptor,
    WrapperKind,
} from '@voml/vos';

export type {
    ClassDescriptor,
    FieldAccess,
    FieldMap,
    ItemKind,
    ItemMetadataKey,
    NamedFieldMap,
    NamedItemDescriptor,
    NamedItemKind,
    ObjectDescriptor,
    RowOf,
    StructuralDescriptor,
    TableDescriptor,
} from '@voml/vos';

export type { ReferenceKey, VosFileRef } from '@voml/vos';
