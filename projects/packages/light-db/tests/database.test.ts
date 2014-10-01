/**
 * 浏览器数据库作者面测试
 */

import { BinaryUtils, BrowserDatabase, LightDbError, LightDbErrorCodes, MemoryStorageBackend } from "../src";
import type { LightDatabaseOptions } from "../src";
import { assertEqual, assertFalse, assertNull, assertThrows, assertTrue, describe } from "./test-utils";

/** 测试用选项 */
function options(backend: MemoryStorageBackend, overrides: LightDatabaseOptions = {}): LightDatabaseOptions {
    return { storage: backend, lease: "none", autoCheckpoint: false, ...overrides };
}

/**
 * 创建数据库测试套件
 */
export function createDatabaseTests() {
    const suite = describe("browser database");

    suite.test("写入、读取与删除在重开后仍然生效", async () => {
        const backend = new MemoryStorageBackend();
        const db = await BrowserDatabase.open("app", options(backend));

        await db.put("alpha", BinaryUtils.encode("one"));
        assertEqual(BinaryUtils.decode((await db.get("alpha"))!), "one");
        assertNull(await db.get("missing"));
        await db.close();

        const reopened = await BrowserDatabase.open("app", options(backend));
        assertEqual(BinaryUtils.decode((await reopened.get("alpha"))!), "one");
        assertTrue(await reopened.delete("alpha"), "existing key should be deleted");
        assertFalse(await reopened.delete("alpha"), "second delete should report false");
        assertNull(await reopened.get("alpha"));
        await reopened.close();
    });

    suite.test("未检查点的写入由 WAL 恢复", async () => {
        const backend = new MemoryStorageBackend();
        const db = await BrowserDatabase.open("app", options(backend));
        await db.put("wal-key", BinaryUtils.encode("durable"));
        assertTrue(db.stats().walBytes > 0, "write should land in WAL before checkpoint");
        await db.close();

        const reopened = await BrowserDatabase.open("app", options(backend));
        assertEqual(BinaryUtils.decode((await reopened.get("wal-key"))!), "durable");
        await reopened.close();
    });

    suite.test("检查点写入快照并清空 WAL", async () => {
        const backend = new MemoryStorageBackend();
        const db = await BrowserDatabase.open("app", options(backend));
        await db.put("a", BinaryUtils.encode("1"));
        await db.put("b", BinaryUtils.encode("2"));

        const result = await db.checkpoint();
        assertTrue(result.bytesWritten > 0, "checkpoint should write bytes");
        assertTrue(result.walBytesReleased > 0, "checkpoint should release WAL bytes");
        assertEqual(db.stats().walBytes, 0);
        await db.close();

        const reopened = await BrowserDatabase.open("app", options(backend));
        assertEqual(BinaryUtils.decode((await reopened.get("a"))!), "1");
        assertEqual(BinaryUtils.decode((await reopened.get("b"))!), "2");
        await reopened.close();
    });

    suite.test("事务回滚丢弃未提交写入", async () => {
        const backend = new MemoryStorageBackend();
        const db = await BrowserDatabase.open("app", options(backend));
        await db.put("base", BinaryUtils.encode("0"));

        await db.begin();
        await db.put("draft", BinaryUtils.encode("1"));
        assertEqual(BinaryUtils.decode((await db.get("draft"))!), "1", "pending write should be visible inside the transaction");
        await db.rollback();

        assertNull(await db.get("draft"));
        await db.close();

        const reopened = await BrowserDatabase.open("app", options(backend));
        assertNull(await reopened.get("draft"));
        assertEqual(BinaryUtils.decode((await reopened.get("base"))!), "0");
        await reopened.close();
    });

    suite.test("事务提交一次性落盘", async () => {
        const backend = new MemoryStorageBackend();
        const db = await BrowserDatabase.open("app", options(backend));

        await db.begin();
        await db.put("x", BinaryUtils.encode("1"));
        await db.put("y", BinaryUtils.encode("2"));
        await db.commit();

        assertEqual(BinaryUtils.decode((await db.get("x"))!), "1");
        assertEqual(db.stats().transactionState, "idle");
        await db.close();

        const reopened = await BrowserDatabase.open("app", options(backend));
        assertEqual(BinaryUtils.decode((await reopened.get("y"))!), "2");
        await reopened.close();
    });

    suite.test("事务中的删除在提交后生效", async () => {
        const backend = new MemoryStorageBackend();
        const db = await BrowserDatabase.open("app", options(backend));
        await db.put("gone", BinaryUtils.encode("x"));

        await db.begin();
        assertTrue(await db.delete("gone"), "delete should see the committed key");
        assertNull(await db.get("gone"));
        await db.commit();

        assertNull(await db.get("gone"));
        await db.close();
    });

    suite.test("事务期间禁止检查点", async () => {
        const backend = new MemoryStorageBackend();
        const db = await BrowserDatabase.open("app", options(backend));
        await db.begin();
        const error = await assertThrows(() => db.checkpoint(), LightDbError);
        assertEqual(error.code, LightDbErrorCodes.unsupported);
        await db.rollback();
        await db.close();
    });

    suite.test("Schema 文档是唯一真相", async () => {
        const backend = new MemoryStorageBackend();
        const db = await BrowserDatabase.open("app", options(backend));
        const doc = "// @yydb-schema-version: 3\nentity User { id: u64 }\n";

        assertNull(await db.schema(), "new database has no schema");
        await db.ensureSchema(doc);

        const stored = await db.schema();
        assertEqual(stored?.version, 3);
        assertEqual(stored?.document, doc);

        await db.ensureSchema(doc);

        const conflict = await assertThrows(
            () => db.ensureSchema("// @yydb-schema-version: 4\nentity User { id: u64 }\n"),
            LightDbError,
        );
        assertEqual(conflict.code, LightDbErrorCodes.schemaConflict);
        await db.close();
    });

    suite.test("Schema 在重开后保持", async () => {
        const backend = new MemoryStorageBackend();
        const doc = "// @yydb-schema-version: 2\nentity Note { id: u64 }\n";
        const db = await BrowserDatabase.open("app", options(backend));
        await db.ensureSchema(doc);
        await db.close();

        const reopened = await BrowserDatabase.open("app", options(backend));
        assertEqual((await reopened.schema())?.document, doc);
        await reopened.close();
    });

    suite.test("拒绝空的与保留前缀的用户键", async () => {
        const backend = new MemoryStorageBackend();
        const db = await BrowserDatabase.open("app", options(backend));

        const empty = await assertThrows(() => db.put("", new Uint8Array([1])), LightDbError);
        assertEqual(empty.code, LightDbErrorCodes.unsupported);

        const reserved = await assertThrows(() => db.put("__yydb__/schema", new Uint8Array([1])), LightDbError);
        assertEqual(reserved.code, LightDbErrorCodes.unsupported);
        await db.close();
    });

    suite.test("关闭后拒绝继续访问", async () => {
        const backend = new MemoryStorageBackend();
        const db = await BrowserDatabase.open("app", options(backend));
        await db.close();

        const error = await assertThrows(() => db.get("a"), LightDbError);
        assertEqual(error.code, LightDbErrorCodes.sessionClosed);
    });

    suite.test("损坏的快照被识别为 yydb.corrupt", async () => {
        const backend = new MemoryStorageBackend();
        const adapter = await backend.openDatabase("app");
        const handle = await adapter.open("main", "readwrite");
        await handle.write(0, new Uint8Array([1, 2, 3]));
        await handle.close();
        await adapter.close();

        const error = await assertThrows(() => BrowserDatabase.open("app", options(backend)), LightDbError);
        assertEqual(error.code, LightDbErrorCodes.corrupt);
    });

    suite.test("统计信息反映记录数与事务状态", async () => {
        const backend = new MemoryStorageBackend();
        const db = await BrowserDatabase.open("app", options(backend));
        await db.put("a", new Uint8Array([1]));
        await db.put("b", new Uint8Array([2]));

        const stats = db.stats();
        assertEqual(stats.recordCount, 2);
        assertEqual(stats.transactionState, "idle");
        assertEqual(stats.storageKind, "memory");
        assertFalse(stats.persistent, "memory backend is not persistent");
        await db.close();
    });

    return suite;
}
