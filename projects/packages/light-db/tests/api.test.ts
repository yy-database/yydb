/**
 * API 层集成测试
 * 测试 LightDB.open/close、collection 方法、事务 API、链式查询构建器
 */

import {
    describe,
    assertEqual,
    assertTrue,
    assertFalse,
    assertNotNull,
    assertNull,
    assertThrows,
    assertNoThrow,
    assertGreater,
    createTempDir,
    removeTempDir,
    TestSuite,
    sleep,
} from "./test-utils";
import { LightDB, openDatabase, defineCollectionSchema } from "../src/api/LightDB";
import { TransactionManager, Transaction, TransactionError } from "../src/api/Transaction";
import { Collection } from "../src/collection/Collection";
import { StorageError } from "../src/errors";

/**
 * 测试文档类型
 */
interface User {
    id: number;
    name: string;
    email: string;
    age: number;
    balance: number;
}

interface Product {
    sku: string;
    name: string;
    price: number;
    stock: number;
}

/**
 * LightDB API 测试套件
 */
function createLightDBAPITests(): TestSuite {
    const suite = describe("LightDB API Tests");

    let tempDir: string;
    let db: LightDB;

    suite.beforeEach(async () => {
        tempDir = createTempDir("lightdb-api-");
    });

    suite.afterEach(async () => {
        if (db) {
            await db.close();
        }
        removeTempDir(tempDir);
    });

    suite.test("打开数据库", async () => {
        db = await LightDB.open(tempDir, { dbName: "testdb" });

        assertTrue(db.isOpen);
        assertEqual(db.dbName, "testdb");
        assertEqual(db.path, tempDir);
    });

    suite.test("打开数据库 - 默认名称", async () => {
        db = await LightDB.open(tempDir);

        assertEqual(db.dbName, "default");
    });

    suite.test("打开数据库 - 自定义配置", async () => {
        db = await LightDB.open(tempDir, {
            dbName: "customdb",
            pageSize: 8192,
            walThreshold: 32 * 1024 * 1024,
            autoCheckpoint: false,
        });

        assertTrue(db.isOpen);

        const config = db.getConfig();
        assertEqual(config.pageSize, 8192);
        assertEqual(config.walThreshold, 32 * 1024 * 1024);
        assertFalse(config.autoCheckpoint);
    });

    suite.test("关闭数据库", async () => {
        db = await LightDB.open(tempDir);
        await db.close();

        assertFalse(db.isOpen);
    });

    suite.test("重复关闭数据库", async () => {
        db = await LightDB.open(tempDir);
        await db.close();
        await db.close();

        assertFalse(db.isOpen);
    });

    suite.test("关闭后操作失败", async () => {
        db = await LightDB.open(tempDir);
        await db.close();

        await assertThrows(
            () => db.collection("users", { primaryKey: "id" }),
            StorageError,
        );
    });

    suite.test("创建集合", async () => {
        db = await LightDB.open(tempDir);

        const users = db.collection<User>("users", {
            primaryKey: "id",
        });

        assertNotNull(users);
        assertEqual(users.Name, "users");
        assertEqual(users.PrimaryKeyField, "id");
    });

    suite.test("获取已存在的集合", async () => {
        db = await LightDB.open(tempDir);

        const users1 = db.collection<User>("users", { primaryKey: "id" });
        const users2 = db.collection<User>("users", { primaryKey: "id" });

        assertEqual(users1, users2);
    });

    suite.test("检查集合是否存在", async () => {
        db = await LightDB.open(tempDir);

        assertFalse(db.hasCollection("users"));

        db.collection<User>("users", { primaryKey: "id" });

        assertTrue(db.hasCollection("users"));
    });

    suite.test("删除集合", async () => {
        db = await LightDB.open(tempDir);

        db.collection<User>("users", { primaryKey: "id" });
        const result = db.dropCollection("users");

        assertTrue(result);
        assertFalse(db.hasCollection("users"));
    });

    suite.test("删除不存在的集合", async () => {
        db = await LightDB.open(tempDir);

        const result = db.dropCollection("nonexistent");

        assertFalse(result);
    });

    suite.test("获取所有集合名称", async () => {
        db = await LightDB.open(tempDir);

        db.collection<User>("users", { primaryKey: "id" });
        db.collection<Product>("products", { primaryKey: "sku" });

        const names = db.getCollectionNames();

        assertEqual(names.length, 2);
        assertTrue(names.includes("users"));
        assertTrue(names.includes("products"));
    });

    suite.test("执行检查点", async () => {
        db = await LightDB.open(tempDir, { autoCheckpoint: false });

        const users = db.collection<User>("users", { primaryKey: "id" });
        users.insert({ id: 1, name: "Alice", email: "alice@example.com", age: 25, balance: 100 });

        const result = await db.checkpoint();

        assertTrue(result.timestamp > 0);
        assertGreater(result.lsn, 0n);
    });

    suite.test("获取数据库统计信息", async () => {
        db = await LightDB.open(tempDir);

        const users = db.collection<User>("users", { primaryKey: "id" });
        users.insert({ id: 1, name: "Alice", email: "alice@example.com", age: 25, balance: 100 });

        const stats = await db.getStats();

        assertEqual(stats.dbName, "default");
        assertEqual(stats.collectionCount, 1);
        assertEqual(stats.collections.length, 1);
    });

    suite.test("获取数据库配置", async () => {
        db = await LightDB.open(tempDir, {
            dbName: "testdb",
            pageSize: 8192,
        });

        const config = db.getConfig();

        assertEqual(config.dbName, "testdb");
        assertEqual(config.pageSize, 8192);
    });

    suite.test("同步数据到磁盘", async () => {
        db = await LightDB.open(tempDir);

        const users = db.collection<User>("users", { primaryKey: "id" });
        users.insert({ id: 1, name: "Alice", email: "alice@example.com", age: 25, balance: 100 });

        await assertNoThrow(() => db.sync());
    });

    suite.test("openDatabase 便捷函数", async () => {
        db = await openDatabase(tempDir, { dbName: "conveniencedb" });

        assertTrue(db.isOpen);
        assertEqual(db.dbName, "conveniencedb");
    });

    suite.test("defineCollectionSchema 辅助函数", () => {
        const schema = defineCollectionSchema<User>({
            schema: {} as User,
            primaryKey: "id",
            indexes: [
                { name: "email_idx", fieldName: "email", unique: true },
            ],
        });

        assertEqual(schema.primaryKey, "id");
        assertEqual(schema.indexes!.length, 1);
    });

    return suite;
}

/**
 * 事务 API 测试套件
 */
function createTransactionTests(): TestSuite {
    const suite = describe("Transaction API Tests");

    let tempDir: string;
    let db: LightDB;

    suite.beforeEach(async () => {
        tempDir = createTempDir("lightdb-tx-");
        db = await LightDB.open(tempDir);

        const users = db.collection<User>("users", {
            primaryKey: "id",
            indexes: [
                { name: "email_idx", fieldName: "email", unique: true },
            ],
        });

        users.insert({ id: 1, name: "Alice", email: "alice@example.com", age: 25, balance: 100 });
        users.insert({ id: 2, name: "Bob", email: "bob@example.com", age: 30, balance: 200 });
    });

    suite.afterEach(async () => {
        if (db) {
            await db.close();
        }
        removeTempDir(tempDir);
    });

    suite.test("执行简单事务", async () => {
        const result = await db.transaction(async (tx) => {
            const users = tx.collection<User>("users");
            const user = await users.findOne({ id: 1 });
            await users.update({ id: 1 }, { balance: user!.balance - 50 });
        });

        assertTrue(result.success);
        assertEqual(result.state, "committed");
    });

    suite.test("事务中插入文档", async () => {
        const result = await db.transaction(async (tx) => {
            const users = tx.collection<User>("users");
            await users.insert({ id: 3, name: "Charlie", email: "charlie@example.com", age: 28, balance: 150 });
        });

        assertTrue(result.success);

        const users = db.collection<User>("users", { primaryKey: "id" });
        assertTrue(users.hasCollection);
        assertEqual(users.Size, 3);
    });

    suite.test("事务中更新文档", async () => {
        const result = await db.transaction(async (tx) => {
            const users = tx.collection<User>("users");
            await users.update({ id: 1 }, { balance: 50 });
        });

        assertTrue(result.success);

        const users = db.collection<User>("users", { primaryKey: "id" });
        const user = users.findByPrimaryKey(1);
        assertEqual(user!.balance, 50);
    });

    suite.test("事务中删除文档", async () => {
        const result = await db.transaction(async (tx) => {
            const users = tx.collection<User>("users");
            await users.delete({ id: 1 });
        });

        assertTrue(result.success);

        const users = db.collection<User>("users", { primaryKey: "id" });
        assertEqual(users.Size, 1);
    });

    suite.test("事务中查询文档", async () => {
        let foundUser: User | null = null;

        const result = await db.transaction(async (tx) => {
            const users = tx.collection<User>("users");
            foundUser = await users.findOne({ id: 1 });
        });

        assertTrue(result.success);
        assertNotNull(foundUser);
        assertEqual(foundUser!.name, "Alice");
    });

    suite.test("事务回滚", async () => {
        const users = db.collection<User>("users", { primaryKey: "id" });
        const originalBalance = users.findByPrimaryKey(1)!.balance;

        const result = await db.transaction(async (tx) => {
            const txUsers = tx.collection<User>("users");
            await txUsers.update({ id: 1 }, { balance: 0 });

            throw new Error("Simulated error");
        });

        assertFalse(result.success);
        assertEqual(result.state, "failed");
        assertNotNull(result.error);

        const user = users.findByPrimaryKey(1);
        assertEqual(user!.balance, originalBalance);
    });

    suite.test("事务超时", async () => {
        const result = await db.transaction(
            async (tx) => {
                await sleep(200);
            },
            { timeout: 50 },
        );

        assertFalse(result.success);
    });

    suite.test("获取不存在的集合", async () => {
        await assertThrows(
            async () => {
                await db.transaction(async (tx) => {
                    tx.collection<User>("nonexistent");
                });
            },
            TransactionError,
        );
    });

    suite.test("事务选项", async () => {
        const result = await db.transaction(
            async (tx) => {
                const users = tx.collection<User>("users");
                await users.findOne({ id: 1 });
            },
            {
                timeout: 10000,
                isolationLevel: "read_committed",
            },
        );

        assertTrue(result.success);
    });

    return suite;
}

/**
 * 链式查询构建器集成测试套件
 */
function createChainQueryBuilderTests(): TestSuite {
    const suite = describe("Chain Query Builder Integration Tests");

    let tempDir: string;
    let db: LightDB;
    let users: Collection<User>;

    suite.beforeEach(async () => {
        tempDir = createTempDir("lightdb-query-");
        db = await LightDB.open(tempDir);

        users = db.collection<User>("users", {
            primaryKey: "id",
            indexes: [
                { name: "name_idx", fieldName: "name" },
                { name: "age_idx", fieldName: "age" },
            ],
        });

        users.insert({ id: 1, name: "Alice", email: "alice@example.com", age: 25, balance: 100 });
        users.insert({ id: 2, name: "Bob", email: "bob@example.com", age: 30, balance: 200 });
        users.insert({ id: 3, name: "Charlie", email: "charlie@example.com", age: 25, balance: 150 });
        users.insert({ id: 4, name: "David", email: "david@example.com", age: 35, balance: 300 });
        users.insert({ id: 5, name: "Eve", email: "eve@example.com", age: 28, balance: 250 });
    });

    suite.afterEach(async () => {
        if (db) {
            await db.close();
        }
        removeTempDir(tempDir);
    });

    suite.test("链式查询 - 基本查询", () => {
        const result = users.query()
            .where({ name: "Alice" })
            .toArray();

        assertEqual(result.docs.length, 1);
        assertEqual(result.docs[0]!.name, "Alice");
    });

    suite.test("链式查询 - 多条件", () => {
        const result = users.query()
            .where({ age: 25 })
            .where({ name: "Alice" })
            .toArray();

        assertEqual(result.docs.length, 1);
        assertEqual(result.docs[0]!.name, "Alice");
    });

    suite.test("链式查询 - 比较操作", () => {
        const result = users.query()
            .gt("age", 28)
            .toArray();

        assertEqual(result.docs.length, 2);
    });

    suite.test("链式查询 - 范围查询", () => {
        const result = users.query()
            .gte("age", 25)
            .lte("age", 30)
            .toArray();

        assertEqual(result.docs.length, 4);
    });

    suite.test("链式查询 - 排序", () => {
        const result = users.query()
            .desc("age")
            .toArray();

        assertEqual(result.docs[0]!.age, 35);
        assertEqual(result.docs[4]!.age, 25);
    });

    suite.test("链式查询 - 分页", () => {
        const result = users.query()
            .asc("id")
            .skip(2)
            .limit(2)
            .toArray();

        assertEqual(result.docs.length, 2);
        assertEqual(result.docs[0]!.id, 3);
        assertEqual(result.docs[1]!.id, 4);
    });

    suite.test("链式查询 - 字段投影", () => {
        const result = users.query()
            .select("id", "name")
            .toArray();

        assertEqual(result.docs.length, 5);
        assertNotNull(result.docs[0]!.id);
        assertNotNull(result.docs[0]!.name);
    });

    suite.test("链式查询 - 逻辑或", () => {
        const result = users.query()
            .or({ name: "Alice" }, { name: "Bob" })
            .toArray();

        assertEqual(result.docs.length, 2);
    });

    suite.test("链式查询 - 逻辑与", () => {
        const result = users.query()
            .and({ age: 25 }, { name: "Alice" })
            .toArray();

        assertEqual(result.docs.length, 1);
    });

    suite.test("链式查询 - 复杂组合", () => {
        const result = users.query()
            .gte("age", 25)
            .lte("age", 30)
            .desc("balance")
            .limit(3)
            .toArray();

        assertEqual(result.docs.length, 3);
        assertEqual(result.docs[0]!.balance, 250);
    });

    suite.test("链式查询 - first", () => {
        const result = users.query()
            .where({ age: 25 })
            .asc("id")
            .first();

        assertNotNull(result);
        assertEqual(result!.name, "Alice");
    });

    suite.test("链式查询 - count", () => {
        const count = users.query()
            .gte("age", 28)
            .count();

        assertEqual(count, 3);
    });

    suite.test("链式查询 - hasMatch", () => {
        assertTrue(users.query().where({ name: "Alice" }).hasMatch());
        assertFalse(users.query().where({ name: "NonExistent" }).hasMatch());
    });

    suite.test("链式查询 - 使用索引", () => {
        const result = users.query()
            .where({ name: "Alice" })
            .toArray();

        assertTrue(result.usedIndex ?? false);
        assertEqual(result.indexName, "name");
    });

    return suite;
}

/**
 * 完整工作流集成测试套件
 */
function createWorkflowTests(): TestSuite {
    const suite = describe("Complete Workflow Integration Tests");

    let tempDir: string;
    let db: LightDB;

    suite.beforeEach(async () => {
        tempDir = createTempDir("lightdb-workflow-");
    });

    suite.afterEach(async () => {
        if (db) {
            await db.close();
        }
        removeTempDir(tempDir);
    });

    suite.test("完整的 CRUD 工作流", async () => {
        db = await LightDB.open(tempDir);

        const users = db.collection<User>("users", {
            primaryKey: "id",
            indexes: [
                { name: "email_idx", fieldName: "email", unique: true },
            ],
        });

        users.insert({ id: 1, name: "Alice", email: "alice@example.com", age: 25, balance: 100 });

        let user = users.findByPrimaryKey(1);
        assertEqual(user!.name, "Alice");

        users.updateOne({ id: 1 }, { $set: { age: 26 } });

        user = users.findByPrimaryKey(1);
        assertEqual(user!.age, 26);

        users.deleteOne({ id: 1 });
        assertNull(users.findByPrimaryKey(1));
    });

    suite.test("数据持久化工作流", async () => {
        db = await LightDB.open(tempDir, { autoCheckpoint: false });

        const users = db.collection<User>("users", { primaryKey: "id" });

        for (let i = 1; i <= 10; i++) {
            users.insert({ id: i, name: `User${i}`, email: `user${i}@example.com`, age: 20 + i, balance: i * 10 });
        }

        await db.checkpoint();
        await db.close();

        db = await LightDB.open(tempDir);
        const users2 = db.collection<User>("users", { primaryKey: "id" });

        assertEqual(users2.Size, 10);
    });

    suite.test("多集合工作流", async () => {
        db = await LightDB.open(tempDir);

        const users = db.collection<User>("users", { primaryKey: "id" });
        const products = db.collection<Product>("products", { primaryKey: "sku" });

        users.insert({ id: 1, name: "Alice", email: "alice@example.com", age: 25, balance: 100 });
        products.insert({ sku: "SKU001", name: "Product 1", price: 50, stock: 100 });

        assertEqual(db.getCollectionNames().length, 2);

        const user = users.findByPrimaryKey(1);
        const product = products.findByPrimaryKey("SKU001");

        assertNotNull(user);
        assertNotNull(product);
    });

    suite.test("事务工作流", async () => {
        db = await LightDB.open(tempDir);

        const users = db.collection<User>("users", { primaryKey: "id" });
        users.insert({ id: 1, name: "Alice", email: "alice@example.com", age: 25, balance: 100 });
        users.insert({ id: 2, name: "Bob", email: "bob@example.com", age: 30, balance: 50 });

        const result = await db.transaction(async (tx) => {
            const txUsers = tx.collection<User>("users");

            const alice = await txUsers.findOne({ id: 1 });
            const bob = await txUsers.findOne({ id: 2 });

            await txUsers.update({ id: 1 }, { balance: alice!.balance - 30 });
            await txUsers.update({ id: 2 }, { balance: bob!.balance + 30 });
        });

        assertTrue(result.success);

        const alice = users.findByPrimaryKey(1);
        const bob = users.findByPrimaryKey(2);

        assertEqual(alice!.balance, 70);
        assertEqual(bob!.balance, 80);
    });

    suite.test("查询和索引工作流", async () => {
        db = await LightDB.open(tempDir);

        const users = db.collection<User>("users", {
            primaryKey: "id",
            indexes: [
                { name: "age_idx", fieldName: "age" },
            ],
        });

        for (let i = 1; i <= 100; i++) {
            users.insert({
                id: i,
                name: `User${i}`,
                email: `user${i}@example.com`,
                age: 20 + (i % 20),
                balance: i * 10,
            });
        }

        const result = users.query()
            .where({ age: 25 })
            .toArray();

        assertGreater(result.docs.length, 0);
        assertTrue(result.usedIndex ?? false);
    });

    return suite;
}

/**
 * 导出所有 API 层测试
 */
export function getAPITests(): TestSuite[] {
    return [
        createLightDBAPITests(),
        createTransactionTests(),
        createChainQueryBuilderTests(),
        createWorkflowTests(),
    ];
}
