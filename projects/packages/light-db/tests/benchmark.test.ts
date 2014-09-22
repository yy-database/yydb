/**
 * 性能基准测试
 * 测试插入性能、查询性能、并发性能
 */

import {
    describe,
    assertEqual,
    assertTrue,
    assertGreater,
    createTempDir,
    removeTempDir,
    TestSuite,
    randomString,
    randomInt,
} from "./test-utils";
import { LightDB } from "../src/api/LightDB";
import { BPlusTree } from "../src/indexes/BPlusTree";
import { PrimaryIndex } from "../src/indexes/PrimaryIndex";
import { SecondaryIndex } from "../src/indexes/SecondaryIndex";
import { IndexManager, CollectionIndexConfig } from "../src/indexes/IndexManager";
import { Collection } from "../src/collection/Collection";
import { StorageEngine, createStorageEngine } from "../src/storage/StorageEngine";

/**
 * 基准测试结果接口
 */
interface BenchmarkResult {
    /** 操作名称 */
    operation: string;
    /** 操作次数 */
    count: number;
    /** 总耗时（毫秒） */
    totalMs: number;
    /** 平均耗时（毫秒） */
    avgMs: number;
    /** 每秒操作数 */
    opsPerSecond: number;
}

/**
 * 运行基准测试
 */
function runBenchmark(
    name: string,
    count: number,
    fn: (i: number) => void | Promise<void>,
): BenchmarkResult {
    const startTime = Date.now();

    for (let i = 0; i < count; i++) {
        const result = fn(i);
        if (result instanceof Promise) {
            throw new Error("Use runAsyncBenchmark for async operations");
        }
    }

    const totalMs = Date.now() - startTime;
    const avgMs = totalMs / count;
    const opsPerSecond = Math.round((count / totalMs) * 1000);

    console.log(`  ${name}: ${count} ops in ${totalMs}ms (${opsPerSecond} ops/s, ${avgMs.toFixed(3)}ms avg)`);

    return {
        operation: name,
        count,
        totalMs,
        avgMs,
        opsPerSecond,
    };
}

/**
 * 运行异步基准测试
 */
async function runAsyncBenchmark(
    name: string,
    count: number,
    fn: (i: number) => Promise<void>,
): Promise<BenchmarkResult> {
    const startTime = Date.now();

    for (let i = 0; i < count; i++) {
        await fn(i);
    }

    const totalMs = Date.now() - startTime;
    const avgMs = totalMs / count;
    const opsPerSecond = Math.round((count / totalMs) * 1000);

    console.log(`  ${name}: ${count} ops in ${totalMs}ms (${opsPerSecond} ops/s, ${avgMs.toFixed(3)}ms avg)`);

    return {
        operation: name,
        count,
        totalMs,
        avgMs,
        opsPerSecond,
    };
}

/**
 * B+ 树性能测试套件
 */
function createBPlusTreeBenchmarkTests(): TestSuite {
    const suite = describe("BPlusTree Benchmark Tests");

    const INSERT_COUNT = 10000;
    const QUERY_COUNT = 10000;

    suite.test("B+ 树插入性能", () => {
        const tree = new BPlusTree<number, string>();

        const result = runBenchmark("BPlusTree Insert", INSERT_COUNT, (i) => {
            tree.insert(i, `value-${i}`);
        });

        assertEqual(tree.Size, INSERT_COUNT);
        assertGreater(result.opsPerSecond, 1000);
    });

    suite.test("B+ 树查找性能", () => {
        const tree = new BPlusTree<number, string>();

        for (let i = 0; i < INSERT_COUNT; i++) {
            tree.insert(i, `value-${i}`);
        }

        const result = runBenchmark("BPlusTree Find", QUERY_COUNT, (i) => {
            tree.find(i % INSERT_COUNT);
        });

        assertGreater(result.opsPerSecond, 1000);
    });

    suite.test("B+ 树删除性能", () => {
        const tree = new BPlusTree<number, string>();

        for (let i = 0; i < INSERT_COUNT; i++) {
            tree.insert(i, `value-${i}`);
        }

        const result = runBenchmark("BPlusTree Delete", INSERT_COUNT / 2, (i) => {
            tree.delete(i * 2);
        });

        assertGreater(result.opsPerSecond, 1000);
    });

    suite.test("B+ 树范围查询性能", () => {
        const tree = new BPlusTree<number, string>();

        for (let i = 0; i < INSERT_COUNT; i++) {
            tree.insert(i, `value-${i}`);
        }

        const result = runBenchmark("BPlusTree Range", 100, () => {
            tree.range(0, 1000);
        });

        assertGreater(result.opsPerSecond, 100);
    });

    suite.test("B+ 树字符串键性能", () => {
        const tree = new BPlusTree<string, number>();
        const keys: string[] = [];

        for (let i = 0; i < INSERT_COUNT; i++) {
            keys.push(`key-${randomString(20)}`);
        }

        const insertResult = runBenchmark("BPlusTree String Insert", INSERT_COUNT, (i) => {
            tree.insert(keys[i]!, i);
        });

        const findResult = runBenchmark("BPlusTree String Find", QUERY_COUNT, (i) => {
            tree.find(keys[i % INSERT_COUNT]!);
        });

        assertGreater(insertResult.opsPerSecond, 500);
        assertGreater(findResult.opsPerSecond, 500);
    });

    return suite;
}

/**
 * 索引性能测试套件
 */
function createIndexBenchmarkTests(): TestSuite {
    const suite = describe("Index Benchmark Tests");

    const INSERT_COUNT = 5000;
    const QUERY_COUNT = 5000;

    interface TestDoc {
        id: number;
        name: string;
        value: number;
    }

    suite.test("主键索引插入性能", () => {
        const index = new PrimaryIndex<TestDoc>({
            name: "pk",
            fieldName: "id",
        });

        const result = runBenchmark("PrimaryIndex Insert", INSERT_COUNT, (i) => {
            index.insert(i, { id: i, name: `name-${i}`, value: i * 10 });
        });

        assertGreater(result.opsPerSecond, 1000);
    });

    suite.test("主键索引查找性能", () => {
        const index = new PrimaryIndex<TestDoc>({
            name: "pk",
            fieldName: "id",
        });

        for (let i = 0; i < INSERT_COUNT; i++) {
            index.insert(i, { id: i, name: `name-${i}`, value: i * 10 });
        }

        const result = runBenchmark("PrimaryIndex Find", QUERY_COUNT, (i) => {
            index.find(i % INSERT_COUNT);
        });

        assertGreater(result.opsPerSecond, 1000);
    });

    suite.test("二级索引插入性能", () => {
        const index = new SecondaryIndex({
            name: "value_idx",
            fieldName: "value",
        });

        const result = runBenchmark("SecondaryIndex Insert", INSERT_COUNT, (i) => {
            index.insert(i * 10, i);
        });

        assertGreater(result.opsPerSecond, 1000);
    });

    suite.test("二级索引查找性能", () => {
        const index = new SecondaryIndex({
            name: "value_idx",
            fieldName: "value",
        });

        for (let i = 0; i < INSERT_COUNT; i++) {
            index.insert(i * 10, i);
        }

        const result = runBenchmark("SecondaryIndex Find", QUERY_COUNT, (i) => {
            index.find((i % INSERT_COUNT) * 10);
        });

        assertGreater(result.opsPerSecond, 1000);
    });

    suite.test("索引管理器插入性能", () => {
        const config: CollectionIndexConfig<TestDoc> = {
            name: "test",
            primaryKey: "id",
            primaryKeyExtractor: (doc) => doc.id,
            secondaryIndexes: [
                {
                    name: "name_idx",
                    fieldName: "name",
                    valueExtractor: (doc) => doc.name,
                },
            ],
        };

        const manager = new IndexManager<TestDoc>(config);

        const result = runBenchmark("IndexManager Insert", INSERT_COUNT, (i) => {
            manager.insert({ id: i, name: `name-${i}`, value: i * 10 });
        });

        assertGreater(result.opsPerSecond, 500);
    });

    return suite;
}

/**
 * 集合性能测试套件
 */
function createCollectionBenchmarkTests(): TestSuite {
    const suite = describe("Collection Benchmark Tests");

    const INSERT_COUNT = 2000;
    const QUERY_COUNT = 2000;

    interface TestDoc {
        id: number;
        name: string;
        age: number;
        email: string;
    }

    let collection: Collection<TestDoc>;

    suite.beforeEach(() => {
        collection = new Collection<TestDoc>({
            name: "test",
            primaryKey: "id",
            indexes: [
                { name: "name_idx", fieldName: "name" },
                { name: "age_idx", fieldName: "age" },
            ],
        });
    });

    suite.test("集合插入性能", () => {
        const result = runBenchmark("Collection Insert", INSERT_COUNT, (i) => {
            collection.insert({
                id: i,
                name: `name-${i}`,
                age: 20 + (i % 50),
                email: `user${i}@example.com`,
            });
        });

        assertGreater(result.opsPerSecond, 500);
    });

    suite.test("集合批量插入性能", () => {
        const docs: TestDoc[] = [];
        for (let i = 0; i < INSERT_COUNT; i++) {
            docs.push({
                id: i,
                name: `name-${i}`,
                age: 20 + (i % 50),
                email: `user${i}@example.com`,
            });
        }

        const startTime = Date.now();
        const result = collection.insertMany(docs);
        const totalMs = Date.now() - startTime;

        console.log(`  Collection InsertMany: ${INSERT_COUNT} docs in ${totalMs}ms (${Math.round((INSERT_COUNT / totalMs) * 1000)} docs/s)`);

        assertTrue(result.success);
    });

    suite.test("集合查找性能", () => {
        for (let i = 0; i < INSERT_COUNT; i++) {
            collection.insert({
                id: i,
                name: `name-${i}`,
                age: 20 + (i % 50),
                email: `user${i}@example.com`,
            });
        }

        const result = runBenchmark("Collection Find", QUERY_COUNT, (i) => {
            collection.findByPrimaryKey(i % INSERT_COUNT);
        });

        assertGreater(result.opsPerSecond, 1000);
    });

    suite.test("集合查询构建器性能", () => {
        for (let i = 0; i < INSERT_COUNT; i++) {
            collection.insert({
                id: i,
                name: `name-${i}`,
                age: 20 + (i % 50),
                email: `user${i}@example.com`,
            });
        }

        const result = runBenchmark("Collection Query", 500, (i) => {
            collection.query()
                .where({ age: 25 + (i % 10) })
                .toArray();
        });

        assertGreater(result.opsPerSecond, 100);
    });

    suite.test("集合更新性能", () => {
        for (let i = 0; i < INSERT_COUNT; i++) {
            collection.insert({
                id: i,
                name: `name-${i}`,
                age: 20 + (i % 50),
                email: `user${i}@example.com`,
            });
        }

        const result = runBenchmark("Collection Update", INSERT_COUNT, (i) => {
            collection.updateOne({ id: i }, { $set: { age: 30 } });
        });

        assertGreater(result.opsPerSecond, 500);
    });

    suite.test("集合删除性能", () => {
        for (let i = 0; i < INSERT_COUNT; i++) {
            collection.insert({
                id: i,
                name: `name-${i}`,
                age: 20 + (i % 50),
                email: `user${i}@example.com`,
            });
        }

        const result = runBenchmark("Collection Delete", INSERT_COUNT / 2, (i) => {
            collection.deleteByPrimaryKey(i);
        });

        assertGreater(result.opsPerSecond, 500);
    });

    return suite;
}

/**
 * 存储引擎性能测试套件
 */
function createStorageEngineBenchmarkTests(): TestSuite {
    const suite = describe("StorageEngine Benchmark Tests");

    const INSERT_COUNT = 500;
    let tempDir: string;

    suite.beforeEach(() => {
        tempDir = createTempDir("storage-bench-");
    });

    suite.afterEach(async () => {
        removeTempDir(tempDir);
    });

    suite.test("存储引擎插入性能", async () => {
        const engine = await createStorageEngine(tempDir, "bench", { autoCheckpoint: false });

        const result = await runAsyncBenchmark("StorageEngine Insert", INSERT_COUNT, async (i) => {
            await engine.insert(`key-${i}`, Buffer.from(`value-${i}`));
        });

        await engine.close();

        assertGreater(result.opsPerSecond, 100);
    });

    suite.test("存储引擎读取性能", async () => {
        const engine = await createStorageEngine(tempDir, "bench", { autoCheckpoint: false });

        for (let i = 0; i < INSERT_COUNT; i++) {
            await engine.insert(`key-${i}`, Buffer.from(`value-${i}`));
        }

        const result = await runAsyncBenchmark("StorageEngine Get", INSERT_COUNT, async (i) => {
            await engine.get(`key-${i % INSERT_COUNT}`);
        });

        await engine.close();

        assertGreater(result.opsPerSecond, 500);
    });

    suite.test("存储引擎批量插入性能", async () => {
        const engine = await createStorageEngine(tempDir, "bench", { autoCheckpoint: false });

        const entries = [];
        for (let i = 0; i < INSERT_COUNT; i++) {
            entries.push({ key: `key-${i}`, value: Buffer.from(`value-${i}`) });
        }

        const startTime = Date.now();
        await engine.insertBatch(entries);
        const totalMs = Date.now() - startTime;

        console.log(`  StorageEngine InsertBatch: ${INSERT_COUNT} docs in ${totalMs}ms (${Math.round((INSERT_COUNT / totalMs) * 1000)} docs/s)`);

        await engine.close();
    });

    suite.test("存储引擎检查点性能", async () => {
        const engine = await createStorageEngine(tempDir, "bench", { autoCheckpoint: false });

        for (let i = 0; i < INSERT_COUNT; i++) {
            await engine.insert(`key-${i}`, Buffer.from(`value-${i}`));
        }

        const startTime = Date.now();
        await engine.checkpoint();
        const totalMs = Date.now() - startTime;

        console.log(`  StorageEngine Checkpoint: ${INSERT_COUNT} records in ${totalMs}ms`);

        await engine.close();
    });

    return suite;
}

/**
 * LightDB 完整性能测试套件
 */
function createLightDBBenchmarkTests(): TestSuite {
    const suite = describe("LightDB Benchmark Tests");

    const INSERT_COUNT = 500;
    let tempDir: string;

    interface TestDoc {
        id: number;
        name: string;
        value: number;
    }

    suite.beforeEach(() => {
        tempDir = createTempDir("lightdb-bench-");
    });

    suite.afterEach(async () => {
        removeTempDir(tempDir);
    });

    suite.test("LightDB 完整插入流程", async () => {
        const db = await LightDB.open(tempDir, { autoCheckpoint: false });
        const collection = db.collection<TestDoc>("test", { primaryKey: "id" });

        const result = await runAsyncBenchmark("LightDB Insert", INSERT_COUNT, async (i) => {
            collection.insert({ id: i, name: `name-${i}`, value: i * 10 });
        });

        await db.close();

        assertGreater(result.opsPerSecond, 200);
    });

    suite.test("LightDB 完整查询流程", async () => {
        const db = await LightDB.open(tempDir, { autoCheckpoint: false });
        const collection = db.collection<TestDoc>("test", {
            primaryKey: "id",
            indexes: [
                { name: "name_idx", fieldName: "name" },
            ],
        });

        for (let i = 0; i < INSERT_COUNT; i++) {
            collection.insert({ id: i, name: `name-${i}`, value: i * 10 });
        }

        const result = await runAsyncBenchmark("LightDB Query", INSERT_COUNT, async (i) => {
            collection.findByPrimaryKey(i % INSERT_COUNT);
        });

        await db.close();

        assertGreater(result.opsPerSecond, 500);
    });

    suite.test("LightDB 事务性能", async () => {
        const db = await LightDB.open(tempDir, { autoCheckpoint: false });
        const collection = db.collection<TestDoc>("test", { primaryKey: "id" });

        for (let i = 0; i < INSERT_COUNT; i++) {
            collection.insert({ id: i, name: `name-${i}`, value: i * 10 });
        }

        const result = await runAsyncBenchmark("LightDB Transaction", 100, async (i) => {
            await db.transaction(async (tx) => {
                const coll = tx.collection<TestDoc>("test");
                await coll.findOne({ id: i % INSERT_COUNT });
            });
        });

        await db.close();

        assertGreater(result.opsPerSecond, 50);
    });

    suite.test("LightDB 持久化性能", async () => {
        const db = await LightDB.open(tempDir, { autoCheckpoint: false });
        const collection = db.collection<TestDoc>("test", { primaryKey: "id" });

        for (let i = 0; i < INSERT_COUNT; i++) {
            collection.insert({ id: i, name: `name-${i}`, value: i * 10 });
        }

        const startTime = Date.now();
        await db.checkpoint();
        const checkpointMs = Date.now() - startTime;

        console.log(`  LightDB Checkpoint: ${INSERT_COUNT} records in ${checkpointMs}ms`);

        await db.close();

        const startTime2 = Date.now();
        const db2 = await LightDB.open(tempDir);
        const recoveryMs = Date.now() - startTime2;

        console.log(`  LightDB Recovery: ${recoveryMs}ms`);

        const collection2 = db2.collection<TestDoc>("test", { primaryKey: "id" });
        assertEqual(collection2.Size, INSERT_COUNT);

        await db2.close();
    });

    return suite;
}

/**
 * 并发性能测试套件
 */
function createConcurrencyBenchmarkTests(): TestSuite {
    const suite = describe("Concurrency Benchmark Tests");

    const INSERT_COUNT = 500;
    let tempDir: string;

    suite.beforeEach(() => {
        tempDir = createTempDir("concurrency-bench-");
    });

    suite.afterEach(() => {
        removeTempDir(tempDir);
    });

    suite.test("并发插入测试", async () => {
        const db = await LightDB.open(tempDir, { autoCheckpoint: false });
        const collection = db.collection<{ id: number; value: string }>("test", { primaryKey: "id" });

        const concurrentCount = 5;
        const perThread = INSERT_COUNT / concurrentCount;

        const startTime = Date.now();

        const promises = [];
        for (let t = 0; t < concurrentCount; t++) {
            const startId = t * perThread;
            promises.push(
                (async () => {
                    for (let i = 0; i < perThread; i++) {
                        collection.insert({ id: startId + i, value: `value-${startId + i}` });
                    }
                })(),
            );
        }

        await Promise.all(promises);

        const totalMs = Date.now() - startTime;
        console.log(`  Concurrent Insert: ${INSERT_COUNT} docs in ${totalMs}ms (${Math.round((INSERT_COUNT / totalMs) * 1000)} docs/s)`);

        assertEqual(collection.Size, INSERT_COUNT);

        await db.close();
    });

    suite.test("并发读写测试", async () => {
        const db = await LightDB.open(tempDir, { autoCheckpoint: false });
        const collection = db.collection<{ id: number; value: number }>("test", { primaryKey: "id" });

        for (let i = 0; i < INSERT_COUNT; i++) {
            collection.insert({ id: i, value: i });
        }

        const startTime = Date.now();

        const promises = [];
        for (let t = 0; t < 5; t++) {
            promises.push(
                (async () => {
                    for (let i = 0; i < 100; i++) {
                        collection.findByPrimaryKey(randomInt(0, INSERT_COUNT - 1));
                    }
                })(),
            );
        }

        for (let t = 0; t < 2; t++) {
            promises.push(
                (async () => {
                    for (let i = 0; i < 50; i++) {
                        const id = randomInt(0, INSERT_COUNT - 1);
                        collection.updateOne({ id }, { $inc: { value: 1 } });
                    }
                })(),
            );
        }

        await Promise.all(promises);

        const totalMs = Date.now() - startTime;
        console.log(`  Concurrent Read/Write: ${totalMs}ms`);

        await db.close();
    });

    suite.test("并发事务测试", async () => {
        const db = await LightDB.open(tempDir, { autoCheckpoint: false });
        const collection = db.collection<{ id: number; balance: number }>("accounts", { primaryKey: "id" });

        for (let i = 0; i < 10; i++) {
            collection.insert({ id: i, balance: 1000 });
        }

        const startTime = Date.now();

        const promises = [];
        for (let t = 0; t < 20; t++) {
            promises.push(
                db.transaction(async (tx) => {
                    const accounts = tx.collection<{ id: number; balance: number }>("accounts");

                    const from = await accounts.findOne({ id: t % 10 });
                    const to = await accounts.findOne({ id: (t + 1) % 10 });

                    if (from && to) {
                        await accounts.update({ id: from.id }, { balance: from.balance - 10 });
                        await accounts.update({ id: to.id }, { balance: to.balance + 10 });
                    }
                }),
            );
        }

        const results = await Promise.all(promises);
        const successCount = results.filter((r) => r.success).length;

        const totalMs = Date.now() - startTime;
        console.log(`  Concurrent Transactions: ${successCount}/20 successful in ${totalMs}ms`);

        await db.close();
    });

    return suite;
}

/**
 * 导出所有性能基准测试
 */
export function getBenchmarkTests(): TestSuite[] {
    return [
        createBPlusTreeBenchmarkTests(),
        createIndexBenchmarkTests(),
        createCollectionBenchmarkTests(),
        createStorageEngineBenchmarkTests(),
        createLightDBBenchmarkTests(),
        createConcurrencyBenchmarkTests(),
    ];
}
