/**
 * 存储适配器与写租约测试
 */

import { acquireWriterLease, LightDbError, LightDbErrorCodes, MemoryStorageBackend } from "../src";
import { assertEqual, assertFalse, assertThrows, assertTrue, describe } from "./test-utils";

/**
 * 创建伪 Web Locks 实现
 */
function createFakeLocks() {
    const held = new Set<string>();
    return {
        async request(
            name: string,
            _options: { mode: "exclusive"; ifAvailable: true },
            callback: (lock: unknown) => Promise<void>,
        ): Promise<void> {
            if (held.has(name)) {
                await callback(null);
                return;
            }
            held.add(name);
            try {
                await callback({ name });
            } finally {
                held.delete(name);
            }
        },
    };
}

/**
 * 创建适配器测试套件
 */
export function createAdapterTests() {
    const suite = describe("storage adapter");

    suite.test("内存适配器按位置读写并扩展文件", async () => {
        const backend = new MemoryStorageBackend();
        const adapter = await backend.openDatabase("adapter");
        const handle = await adapter.open("wal", "readwrite");

        await handle.write(4, new Uint8Array([1, 2, 3]));
        assertEqual(await handle.size(), 7);
        assertEqual(Array.from(await handle.read(0, 7)), [0, 0, 0, 0, 1, 2, 3]);

        await handle.truncate(2);
        assertEqual(await handle.size(), 2);
        assertEqual(Array.from(await handle.read(0, 2)), [0, 0]);

        await handle.close();
        await adapter.close();
    });

    suite.test("内存适配器拒绝读取不存在的文件", async () => {
        const backend = new MemoryStorageBackend();
        const adapter = await backend.openDatabase("missing");
        const error = await assertThrows(() => adapter.open("main", "read"), LightDbError);
        assertEqual(error.code, LightDbErrorCodes.io);
        await adapter.close();
    });

    suite.test("内存适配器重命名与列出文件", async () => {
        const backend = new MemoryStorageBackend();
        const adapter = await backend.openDatabase("rename");
        const handle = await adapter.open("main.tmp", "readwrite");
        await handle.write(0, new Uint8Array([9, 9]));
        await handle.close();

        await adapter.rename("main.tmp", "main");
        assertFalse(await adapter.exists("main.tmp"), "source file should be gone");
        assertTrue(await adapter.exists("main"), "target file should exist");
        assertEqual(await adapter.list(), ["main"]);
        await adapter.close();
    });

    suite.test("同一后端重开可以读到上一会话的数据", async () => {
        const backend = new MemoryStorageBackend();
        const first = await backend.openDatabase("persist");
        const handle = await first.open("main", "readwrite");
        await handle.write(0, new Uint8Array([5, 6, 7]));
        await handle.close();
        await first.close();

        const second = await backend.openDatabase("persist");
        const reopened = await second.open("main", "read");
        assertEqual(Array.from(await reopened.read(0, 3)), [5, 6, 7]);
        await reopened.close();
        await second.close();
    });

    suite.test("适配器关闭后拒绝继续访问", async () => {
        const backend = new MemoryStorageBackend();
        const adapter = await backend.openDatabase("closed");
        await adapter.close();
        const error = await assertThrows(() => adapter.open("main"), LightDbError);
        assertEqual(error.code, LightDbErrorCodes.sessionClosed);
    });

    suite.test("缺少 Web Locks 时返回结构化能力缺失错误", async () => {
        const error = await assertThrows(() => acquireWriterLease("db", "web-locks", null), LightDbError);
        assertEqual(error.code, LightDbErrorCodes.leaseUnavailable);
        assertTrue(error.retryable, "leaseUnavailable should be retryable");
    });

    suite.test("同一数据库只允许一个写租约", async () => {
        const locks = createFakeLocks();
        const lease = await acquireWriterLease("db", "web-locks", locks);

        const error = await assertThrows(() => acquireWriterLease("db", "web-locks", locks), LightDbError);
        assertEqual(error.code, LightDbErrorCodes.leaseUnavailable);

        await lease.release();

        const next = await acquireWriterLease("db", "web-locks", locks);
        await next.release();
    });

    suite.test("显式 none 模式不获取宿主锁", async () => {
        const lease = await acquireWriterLease("db", "none");
        await lease.release();
    });

    return suite;
}
