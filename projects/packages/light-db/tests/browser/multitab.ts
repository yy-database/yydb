/**
 * 多标签页用例的页面侧动作
 *
 * 由 `run.ts` 在同一个浏览器上下文的两个标签页里分别调用，
 * 验证写租约互斥、关闭通知与重开后的一致性。
 */

import { BrowserDatabase as Database } from '../../src/api/database';
import { OpfsStorageAdapterFactory } from '../../src/storage/opfs/OpfsStorageAdapter';
import { LightDbError } from '../../src/errors/index';
import { listenDatabaseEvents } from '../../src/core/index';

/** 页面内保留的数据库与监听器 */
interface MultitabScope {
    __yydbEvents: string[];
    __yydbHeld: Database | null;
    __yydbUnsubscribe: (() => void) | null;
}

/** 尝试打开的结果 */
export interface OpenAttempt {
    /** 是否打开成功 */
    ok: boolean;
    /** 失败错误码 */
    code?: string;
    /** 是否可重试 */
    retryable?: boolean;
    /** 打开后读到的值 */
    value?: string | null;
}

/**
 * 在当前标签页监听数据库事件
 * @param name 数据库名
 */
export function listenDatabase(name: string): void {
    const scope = scopeOf();
    scope.__yydbUnsubscribe?.();
    scope.__yydbEvents = [];
    scope.__yydbUnsubscribe = listenDatabaseEvents(name, (event) => {
        scope.__yydbEvents.push(event.type);
    });
}

/**
 * 读取已收集的事件类型
 */
export function collectedEvents(): string[] {
    return [...scopeOf().__yydbEvents];
}

/**
 * 打开数据库、写入并保持打开
 * @param name 数据库名
 * @param key 键
 * @param value 值
 */
export async function openAndHold(name: string, key: string, value: string): Promise<OpenAttempt> {
    const scope = scopeOf();
    try {
        const db = await Database.open(name, { storage: new OpfsStorageAdapterFactory() });
        await db.put(key, encode(value));
        scope.__yydbHeld = db;
        return { ok: true };
    } catch (error) {
        return failure(error);
    }
}

/**
 * 在持有的数据库上再写一次
 * @param key 键
 * @param value 值
 */
export async function writeHeld(key: string, value: string): Promise<OpenAttempt> {
    const db = scopeOf().__yydbHeld;
    if (!db) {
        return { ok: false, code: 'no-held-database' };
    }
    try {
        await db.put(key, encode(value));
        return { ok: true };
    } catch (error) {
        return failure(error);
    }
}

/**
 * 关闭当前标签页持有的数据库
 */
export async function closeHeld(): Promise<void> {
    const scope = scopeOf();
    const db = scope.__yydbHeld;
    scope.__yydbHeld = null;
    if (db) {
        await db.close();
    }
}

/**
 * 尝试打开数据库并读取一个键
 * @param name 数据库名
 * @param key 键
 */
export async function tryOpenAndRead(name: string, key: string): Promise<OpenAttempt> {
    try {
        const db = await Database.open(name, { storage: new OpfsStorageAdapterFactory() });
        const value = await db.get(key);
        await db.close();
        return { ok: true, value: value ? new TextDecoder().decode(value) : null };
    } catch (error) {
        return failure(error);
    }
}

/**
 * 打开数据库、写入并关闭
 * @param name 数据库名
 * @param key 键
 * @param value 值
 */
export async function openWriteClose(name: string, key: string, value: string): Promise<OpenAttempt> {
    try {
        const db = await Database.open(name, { storage: new OpfsStorageAdapterFactory() });
        await db.put(key, encode(value));
        await db.close();
        return { ok: true };
    } catch (error) {
        return failure(error);
    }
}

/**
 * 转换为结构化失败
 *
 * browser 环境下 `instanceof LightDbError` 可能因模块加载路径不同而不成立，
 * 这里用 duck-typing 检查 `code` 属性是否存在且为字符串。
 */
function failure(error: unknown): OpenAttempt {
    if (error && typeof error === 'object') {
        const obj = error as Record<string, unknown>;
        if (typeof obj['code'] === 'string') {
            return {
                ok: false,
                code: obj['code'] as string,
                retryable: typeof obj['retryable'] === 'boolean' ? (obj['retryable'] as boolean) : undefined,
            };
        }
    }
    return { ok: false, code: `unexpected: ${String(error)}` };
}

/**
 * 取得页面级状态
 */
function scopeOf(): MultitabScope {
    const scope = globalThis as unknown as MultitabScope;
    scope.__yydbEvents = scope.__yydbEvents ?? [];
    scope.__yydbHeld = scope.__yydbHeld ?? null;
    scope.__yydbUnsubscribe = scope.__yydbUnsubscribe ?? null;
    return scope;
}

/**
 * 文本编码
 * @param text 文本
 */
function encode(text: string): Uint8Array {
    return new TextEncoder().encode(text);
}
