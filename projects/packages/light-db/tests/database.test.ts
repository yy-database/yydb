/**
 * 与 `@yydb/yydb` 同构的数据库句柄测试
 * 覆盖 KV 读写、Schema 持久化、保留键与重开行为
 */

import {
    describe,
    assertEqual,
    assertTrue,
    assertFalse,
    assertNull,
    assertThrows,
    createTempDir,
    removeTempDir,
    TestSuite,
} from "./test-utils";
import { Database, documentVersion } from "../src/api/Database";
import { LightDbError, LightDbErrorCodes } from "../src/errors";

/**
 * Schema 文档示例
 */
const SCHEMA_V1 = `// @yydb-schema-version: 1
type User {
    id: Int
}`;

/**
 * 更新后的 Schema 文档示例
 */
const SCHEMA_V2 = `// @yydb-schema-version: 2
type User {
    id: Int
    name: String
}`;

/**
 * LightDB 同构句柄测试套件
 */
function createDatabaseTests(): TestSuite {
    const suite = describe("Database Facade Tests");

    let tempDir: string;
    let db: Database;

    suite.beforeEach(async () => {
        tempDir = createTempDir("lightdb-database-");
    });

    suite.afterEach(async () => {
        if (db) {
            await db.close();
        }
        removeTempDir(tempDir);
    });

    suite.test("打开数据库", async () => {
        db = await Database.open(tempDir, { dbName: "isomorphic" });

        assertTrue(db.isOpen);
        assertEqual(db.dbName, "isomorphic");
    });

    suite.test("读写二进制值", async () => {
        db = await Database.open(tempDir);

        await db.put("user:1", "Ada");
        const value = await db.get("user:1");

        assertEqual(new TextDecoder().decode(value!), "Ada");
    });

    suite.test("读写 Uint8Array 值", async () => {
        db = await Database.open(tempDir);

        await db.put("blob", new Uint8Array([1, 2, 3]));

        assertEqual(Array.from((await db.get("blob"))!), [1, 2, 3]);
    });

    suite.test("读取不存在的键", async () => {
        db = await Database.open(tempDir);

        assertNull(await db.get("missing"));
    });

    suite.test("删除键", async () => {
        db = await Database.open(tempDir);

        await db.put("user:1", "Ada");

        assertTrue(await db.delete("user:1"));
        assertFalse(await db.delete("user:1"));
        assertNull(await db.get("user:1"));
    });

    suite.test("拒绝内部保留键", async () => {
        db = await Database.open(tempDir);

        await assertThrows(() => db.put("__yydb__/schema", "x"), LightDbError);
    });

    suite.test("读取未持久化的 Schema", async () => {
        db = await Database.open(tempDir);

        assertNull(await db.getSchema());
        assertNull(await db.schemaVersion());
    });

    suite.test("持久化 Schema 并读取版本", async () => {
        db = await Database.open(tempDir);

        await db.ensureSchema(1, SCHEMA_V1);

        const schema = await db.getSchema();

        assertEqual(schema!.version, 1);
        assertEqual(schema!.document, SCHEMA_V1);
        assertEqual(await db.schemaVersion(), 1);
    });

    suite.test("重复持久化相同 Schema", async () => {
        db = await Database.open(tempDir);

        await db.ensureSchema(1, SCHEMA_V1);
        await db.ensureSchema(1, SCHEMA_V1);

        assertEqual((await db.getSchema())!.document, SCHEMA_V1);
    });

    suite.test("Schema 文档变化时报告冲突", async () => {
        db = await Database.open(tempDir);

        await db.ensureSchema(1, SCHEMA_V1);

        const error = await assertThrows(() => db.ensureSchema(2, SCHEMA_V2), LightDbError);

        assertEqual(error.code, LightDbErrorCodes.schemaConflict);
    });

    suite.test("Schema 文档是版本真相", () => {
        assertEqual(documentVersion(SCHEMA_V1), 1);
        assertEqual(documentVersion(SCHEMA_V2), 2);
        assertEqual(documentVersion("type User {\n    id: Int\n}"), 1);
    });

    suite.test("统计用户键数量", async () => {
        db = await Database.open(tempDir);

        await db.put("user:1", "Ada");
        await db.put("user:2", "Bob");
        await db.ensureSchema(1, SCHEMA_V1);

        assertEqual(await db.recordCount(), 2);
    });

    suite.test("检查点后重开保留数据", async () => {
        db = await Database.open(tempDir, { autoCheckpoint: false });

        await db.put("user:1", "Ada");
        await db.ensureSchema(1, SCHEMA_V1);
        await db.checkpoint();
        await db.close();

        db = await Database.open(tempDir);
        const value = await db.get("user:1");

        assertEqual(new TextDecoder().decode(value!), "Ada");
        assertEqual((await db.getSchema())!.version, 1);
    });

    suite.test("关闭后拒绝访问", async () => {
        db = await Database.open(tempDir);
        await db.close();

        const error = await assertThrows(() => db.get("user:1"), LightDbError);

        assertEqual(error.code, LightDbErrorCodes.sessionClosed);
    });

    return suite;
}

export { createDatabaseTests };
