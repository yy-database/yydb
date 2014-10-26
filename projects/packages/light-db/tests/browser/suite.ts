/**
 * 浏览器端一致性套件
 *
 * 在真实浏览器里验证 OPFS 持久化、WAL 恢复、检查点、损坏检测和写租约。
 * 这里的代码只依赖 Web 平台，不引入任何 Node 内置模块。
 */

import {
    Database,
    isOpfsAvailable,
    LightDbError,
    LightDbErrorCodes,
    listenDatabaseEvents,
    OpfsStorageAdapterFactory,
} from '../../src/index';

/** 单个用例结果 */
export interface BrowserCaseResult {
    /** 用例名 */
    name: string;
    /** 是否通过 */
    ok: boolean;
    /** 失败描述 */
    error?: string;
}

/** 应用 DOM 或 Web 平台的最小结构化视图 */
interface WebScope {
    navigator: {
        storage?: {
            getDirectory(): Promise<OpfsDirectoryLike>;
        };
    };
}

/** 浏览器 OPFS 目录的最小结构化视图（测试白盒用） */
interface OpfsDirectoryLike {
    getDirectoryHandle(name: string, options?: { create?: boolean }): Promise<OpfsDirectoryLike>;
    getFileHandle(name: string, options?: { create?: boolean }): Promise<OpfsFileHandleLike>;
    removeEntry(name: string, options?: { recursive?: boolean }): Promise<void>;
}

/** 浏览器 OPFS 文件句柄的最小结构化视图 */
interface OpfsFileHandleLike {
    createWritable(options?: { keepExistingData?: boolean }): Promise<OpfsWritableLike>;
}

/** 可写流 */
interface OpfsWritableLike {
    write(options: { type: 'write'; position: number; data: Uint8Array }): Promise<void>;
    close(): Promise<void>;
}

/** 逻辑快照文件名，与 `OpfsStorageAdapter` 保持一致 */
const SNAPSHOT_FILE = 'main.snapshot';

/**
 * 运行浏览器套件
 * @returns 用例结果
 */
export async function runBrowserSuite(): Promise<BrowserCaseResult[]> {
    const results: BrowserCaseResult[] = [];

    await runCase(results, 'OPFS 持久化重开保留数据与 schema', async () => {
        if (!isOpfsAvailable()) {
            throw new Error('当前宿主没有 OPFS');
        }

        const name = uniqueName('persist');
        try {
            const db = await open(name);
            await db.ensureSchema('// @yydb-schema-version: 2\nkv users');
            await db.put('greeting', encode('你好，浏览器'));
            await db.close();

            const reopened = await open(name);
            const value = await reopened.get('greeting');
            const schema = await reopened.schema();
            await reopened.close();

            assertBytes(value, encode('你好，浏览器'), '重开后读到的值不一致');
            assertEqual(schema?.version, 2, 'schema 版本未保留');
            assertEqual(schema?.document, '// @yydb-schema-version: 2\nkv users', 'schema 文档未保留');
        } finally {
            await dropDatabase(name);
        }
    });

    await runCase(results, '未经检查点的写入由 WAL 恢复', async () => {
        const name = uniqueName('wal');
        try {
            const db = await open(name);
            await db.put('a', encode('1'));
            await db.put('b', encode('2'));
            const beforeClose = db.stats();
            await db.close();

            assertTrue(beforeClose.walBytes > 0, '写入应立即进入 WAL');
            assertEqual(beforeClose.storageKind, 'opfs', '默认后端应为 OPFS');

            const reopened = await open(name);
            assertBytes(await reopened.get('a'), encode('1'), 'WAL 未恢复第一处写入');
            assertBytes(await reopened.get('b'), encode('2'), 'WAL 未恢复第二处写入');
            await reopened.close();
        } finally {
            await dropDatabase(name);
        }
    });

    await runCase(results, '检查点写入快照并清空 WAL', async () => {
        const name = uniqueName('checkpoint');
        try {
            const db = await open(name);
            await db.put('stable', encode('value'));
            const result = await db.checkpoint();
            const afterCheckpoint = db.stats();

            assertTrue(result.walBytesReleased > 0, '检查点应报告释放的 WAL 字节');
            assertEqual(afterCheckpoint.walBytes, 0, '检查点后 WAL 应为空');
            await db.close();

            const reopened = await open(name);
            assertBytes(await reopened.get('stable'), encode('value'), '快照未保留记录');
            await reopened.close();
        } finally {
            await dropDatabase(name);
        }
    });

    await runCase(results, '损坏的快照在浏览器中报告 yydb.corrupt', async () => {
        const name = uniqueName('corrupt');
        try {
            const db = await open(name);
            await db.put('key', encode('value'));
            await db.checkpoint();
            await db.close();

            await corruptSnapshot(name);

            const failure = await expectFailure(() => open(name));
            assertEqual(failure.code, LightDbErrorCodes.corrupt, '损坏快照未映射为 yydb.corrupt');
        } finally {
            await dropDatabase(name);
        }
    });

    await runCase(results, '同一数据库只允许一个浏览器写者', async () => {
        const name = uniqueName('lease');
        try {
            const first = await open(name);
            const failure = await expectFailure(() => open(name));

            assertEqual(failure.code, LightDbErrorCodes.leaseUnavailable, '第二个写者未得到结构化错误');
            assertTrue(failure.retryable, '写租约冲突应标记为可重试');

            await first.put('x', encode('1'));
            await first.close();

            const second = await open(name);
            assertBytes(await second.get('x'), encode('1'), '前一个写者关闭后应能读到已提交数据');
            await second.close();
        } finally {
            await dropDatabase(name);
        }
    });

    await runCase(results, '事务在 OPFS 上回滚与提交', async () => {
        const name = uniqueName('tx');
        try {
            const db = await open(name);
            await db.begin();
            await db.put('discarded', encode('1'));
            await db.rollback();
            assertNull(await db.get('discarded'), '回滚的写入不应可见');

            await db.begin();
            await db.put('kept', encode('2'));
            await db.commit();
            await db.close();

            const reopened = await open(name);
            assertBytes(await reopened.get('kept'), encode('2'), '提交的写入应在重开后可见');
            await reopened.close();
        } finally {
            await dropDatabase(name);
        }
    });

    await runCase(results, 'schema 变更广播给同源其它监听者', async () => {
        const name = uniqueName('schema');
        const events: string[] = [];
        const unsubscribe = listenDatabaseEvents(name, (event) => {
            events.push(event.type);
        });

        try {
            const db = await open(name);
            await db.ensureSchema('// @yydb-schema-version: 1\nkv users');
            await waitFor(() => events.includes('schema'), 2000);
            await db.close();
            await waitFor(() => events.includes('close'), 2000);

            assertTrue(events.includes('schema'), '未收到 schema 事件');
            assertTrue(events.includes('close'), '未收到 close 事件');
        } finally {
            unsubscribe();
            await dropDatabase(name);
        }
    });

    await runCase(results, '关闭后的会话发出 session.closed', async () => {
        const name = uniqueName('closed');
        try {
            const db = await open(name);
            await db.close();
            const failure = await expectFailure(() => db.get('any'));
            assertEqual(failure.code, LightDbErrorCodes.sessionClosed, '关闭后应返回 session.closed');
        } finally {
            await dropDatabase(name);
        }
    });

    return results;
}

/**
 * 打开数据库，默认使用 OPFS
 * @param name 数据库名
 */
async function open(name: string): Promise<Database> {
    return Database.open(name, { storage: new OpfsStorageAdapterFactory() });
}

/**
 * 生成唯一数据库名，避免测试之间互相影响
 * @param prefix 前缀
 */
function uniqueName(prefix: string): string {
    return `test-${prefix}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

/**
 * 删除测试数据库所在的 OPFS 目录
 * @param name 数据库名
 */
async function dropDatabase(name: string): Promise<void> {
    const root = await webScope().navigator.storage?.getDirectory();
    if (!root) {
        return;
    }
    try {
        await root.removeEntry(encodeURIComponent(name), { recursive: true });
    } catch {
        // 数据库从未落盘时目录不存在
    }
}

/**
 * 破坏已检查点快照的头部
 * @param name 数据库名
 */
async function corruptSnapshot(name: string): Promise<void> {
    const root = await webScope().navigator.storage?.getDirectory();
    if (!root) {
        throw new Error('当前宿主没有 OPFS');
    }

    const directory = await root.getDirectoryHandle(encodeURIComponent(name));
    const file = await directory.getFileHandle(SNAPSHOT_FILE);
    const writable = await file.createWritable({ keepExistingData: true });
    try {
        await writable.write({ type: 'write', position: 0, data: new Uint8Array([0, 0, 0, 0, 0, 0, 0, 0]) });
    } finally {
        await writable.close();
    }
}

/**
 * 执行并捕获结构化失败
 * @param action 待执行动作
 */
async function expectFailure(action: () => Promise<unknown>): Promise<LightDbError> {
    try {
        await action();
    } catch (error) {
        if (error instanceof LightDbError) {
            return error;
        }
        throw new Error(`期望 LightDbError，实际为 ${String(error)}`);
    }
    throw new Error('期望失败，实际成功');
}

/**
 * 等待条件成立
 * @param predicate 条件
 * @param timeoutMs 超时毫秒
 */
async function waitFor(predicate: () => boolean, timeoutMs: number): Promise<void> {
    const started = Date.now();
    while (!predicate()) {
        if (Date.now() - started > timeoutMs) {
            throw new Error('等待条件超时');
        }
        await new Promise((resolve) => setTimeout(resolve, 10));
    }
}

/**
 * 按用例运行
 * @param results 结果收集
 * @param name 用例名
 * @param body 用例体
 */
async function runCase(results: BrowserCaseResult[], name: string, body: () => Promise<void>): Promise<void> {
    try {
        await body();
        results.push({ name, ok: true });
    } catch (error) {
        results.push({ name, ok: false, error: error instanceof Error ? error.message : String(error) });
    }
}

/**
 * 取得 Web 平台范围
 */
function webScope(): WebScope {
    return globalThis as unknown as WebScope;
}

/**
 * 文本编码
 * @param text 文本
 */
function encode(text: string): Uint8Array {
    return new TextEncoder().encode(text);
}

/**
 * 断言相等
 * @param actual 实际值
 * @param expected 期望值
 * @param message 失败描述
 */
function assertEqual(actual: unknown, expected: unknown, message: string): void {
    if (actual !== expected) {
        throw new Error(`${message}：期望 ${String(expected)}，实际 ${String(actual)}`);
    }
}

/**
 * 断言为真
 * @param value 值
 * @param message 失败描述
 */
function assertTrue(value: boolean, message: string): void {
    if (!value) {
        throw new Error(message);
    }
}

/**
 * 断言为 null
 * @param value 值
 * @param message 失败描述
 */
function assertNull(value: unknown, message: string): void {
    if (value !== null) {
        throw new Error(message);
    }
}

/**
 * 断言字节相等
 * @param actual 实际值
 * @param expected 期望值
 * @param message 失败描述
 */
function assertBytes(actual: Uint8Array | null, expected: Uint8Array, message: string): void {
    if (!actual || actual.length !== expected.length) {
        throw new Error(message);
    }
    for (let index = 0; index < expected.length; index += 1) {
        if (actual[index] !== expected[index]) {
            throw new Error(message);
        }
    }
}
