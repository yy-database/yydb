/**
 * Schema 语义
 *
 * 与 `@yydb/yydb` 的 Rust 实现同构：Schema 文档是唯一真相，
 * 版本由文档前导注释 `// @yydb-schema-version: <n>` 决定，默认 1。
 */

import { BinaryUtils } from '../codec';
import { LightDbError, LightDbErrorCodes } from '../errors';
import type { SchemaVersion } from './types';

/** Schema 记录在数据库中的保留键 */
export const SCHEMA_KEY = '__yydb__/schema';

/** 用户键禁止使用的内部前缀 */
export const RESERVED_KEY_PREFIX = '__yydb__/';

/** 版本声明注释 */
const VERSION_PRAGMA = /^\s*\/\/\s*@yydb-schema-version:\s*(\d+)\s*$/m;

/**
 * 从文档中提取版本
 * @param document Schema 文档
 */
export function documentVersion(document: string): number {
    const match = VERSION_PRAGMA.exec(document);
    if (!match) {
        return 1;
    }
    const version = Number.parseInt(match[1]!, 10);
    return Number.isFinite(version) && version > 0 ? version : 1;
}

/**
 * 校验 Schema 文档
 * @param document Schema 文档
 * @returns 文档声明的版本
 */
export function parseSchemaDocument(document: string): number {
    if (document.trim().length === 0) {
        throw new LightDbError(LightDbErrorCodes.schemaInvalid, 'schema document must not be empty', {
            operation: 'ensureSchema',
        });
    }
    return documentVersion(document);
}

/**
 * 编码 Schema 记录
 * @param schema Schema 版本与文档
 */
export function encodeSchema(schema: SchemaVersion): Uint8Array {
    return BinaryUtils.encode(JSON.stringify(schema));
}

/**
 * 解码 Schema 记录
 * @param value 记录值
 */
export function decodeSchema(value: Uint8Array): SchemaVersion {
    let parsed: unknown;
    try {
        parsed = JSON.parse(BinaryUtils.decode(value));
    } catch (error) {
        throw new LightDbError(LightDbErrorCodes.corrupt, 'persisted schema record is not valid JSON', {
            operation: 'schema',
            cause: error,
        });
    }

    if (!isSchemaVersion(parsed)) {
        throw new LightDbError(LightDbErrorCodes.corrupt, 'persisted schema record has an unexpected shape', {
            operation: 'schema',
        });
    }

    return parsed;
}

/**
 * 判断值是否为 Schema 记录
 * @param value 待判断值
 */
function isSchemaVersion(value: unknown): value is SchemaVersion {
    if (typeof value !== 'object' || value === null) {
        return false;
    }
    const record = value as { version?: unknown; document?: unknown };
    return typeof record.version === 'number' && typeof record.document === 'string';
}
