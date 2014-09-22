/**
 * 存储引擎单元测试
 * 测试 .light 文件、WAL 文件、SHM 文件、检查点和恢复机制
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
    randomString,
} from "./test-utils";
import { StorageEngine, createStorageEngine } from "../src/storage/StorageEngine";
import { LightFileManager, LightFileSerializer, PageManager } from "../src/storage/LightFile";
import { WalManager, WalWriter, WalReader, WalRecordSerializer } from "../src/storage/WALFile";
import { WalIndexManager, ShmFileManager, ShmHashTable, ShmSerializer } from "../src/storage/SHMFile";
import { CheckpointManager, AutoCheckpointScheduler } from "../src/storage/Checkpoint";
import { RecoveryManager, RecoveryValidator, RecoveryPhase } from "../src/storage/Recovery";
import { OperationType, LIGHT_MAGIC, LIGHT_VERSION, DEFAULT_PAGE_SIZE } from "../src/constants";
import { StorageError, FileFormatError, ChecksumError } from "../src/errors";
import * as fs from "fs";
import * as path from "path";

/**
 * LightFile 测试套件
 */
function createLightFileTests(): TestSuite {
    const suite = describe("LightFile Tests");

    let tempDir: string;
    let fileManager: LightFileManager;

    suite.beforeEach(async () => {
        tempDir = createTempDir("lightfile-");
        fileManager = new LightFileManager(tempDir, "test", DEFAULT_PAGE_SIZE);
    });

    suite.afterEach(async () => {
        if (fileManager) {
            await fileManager.close();
        }
        removeTempDir(tempDir);
    });

    suite.test("LightFileSerializer - 序列化和反序列化头部", () => {
        const serializer = new LightFileSerializer(DEFAULT_PAGE_SIZE);

        const header = {
            magic: LIGHT_MAGIC,
            version: LIGHT_VERSION,
            pageSize: DEFAULT_PAGE_SIZE,
            lastSnapshotLsn: 100n,
        };

        const buffer = serializer.serializeHeader(header);
        assertEqual(buffer.length, 36, "Header buffer size should be 36 bytes");

        const deserialized = serializer.deserializeHeader(buffer);
        assertEqual(deserialized.magic, LIGHT_MAGIC);
        assertEqual(deserialized.version, LIGHT_VERSION);
        assertEqual(deserialized.pageSize, DEFAULT_PAGE_SIZE);
        assertEqual(deserialized.lastSnapshotLsn, 100n);
    });

    suite.test("LightFileSerializer - 序列化和反序列化记录", () => {
        const serializer = new LightFileSerializer(DEFAULT_PAGE_SIZE);

        const record = {
            key: "test-key",
            value: Buffer.from("test-value"),
        };

        const buffer = serializer.serializeRecord(record);
        assertTrue(buffer.length > 0, "Record buffer should not be empty");

        const result = serializer.deserializeRecord(buffer);
        assertEqual(result.record.key, "test-key");
        assertEqual(result.record.value.toString(), "test-value");
        assertEqual(result.bytesRead, buffer.length);
    });

    suite.test("LightFileSerializer - 序列化和反序列化页面", () => {
        const serializer = new LightFileSerializer(DEFAULT_PAGE_SIZE);

        const page = {
            pageId: 0,
            records: [
                { key: "key1", value: Buffer.from("value1") },
                { key: "key2", value: Buffer.from("value2") },
            ],
            checksum: 0,
        };

        const buffer = serializer.serializePage(page);
        assertEqual(buffer.length, DEFAULT_PAGE_SIZE, "Page buffer size should match page size");

        const deserialized = serializer.deserializePage(buffer, 0);
        assertEqual(deserialized.pageId, 0);
        assertEqual(deserialized.records.length, 2);
        assertEqual(deserialized.records[0]!.key, "key1");
        assertEqual(deserialized.records[1]!.key, "key2");
    });

    suite.test("LightFileManager - 创建和打开文件", async () => {
        await fileManager.open();
        assertTrue(await fileManager.exists(), "File should exist after open");
        assertEqual(fileManager.getPageSize(), DEFAULT_PAGE_SIZE);
    });

    suite.test("LightFileManager - 写入和读取头部", async () => {
        await fileManager.open();

        const header = {
            magic: LIGHT_MAGIC,
            version: LIGHT_VERSION,
            pageSize: DEFAULT_PAGE_SIZE,
            lastSnapshotLsn: 200n,
        };

        await fileManager.writeHeader(header);
        const readHeader = await fileManager.readHeader();

        assertNotNull(readHeader);
        assertEqual(readHeader!.magic, LIGHT_MAGIC);
        assertEqual(readHeader!.lastSnapshotLsn, 200n);
    });

    suite.test("LightFileManager - 写入和读取页面", async () => {
        await fileManager.open();

        const page = {
            pageId: 0,
            records: [{ key: "test-key", value: Buffer.from("test-value") }],
            checksum: 0,
        };

        await fileManager.writePage(page);
        const readPage = await fileManager.readPage(0);

        assertNotNull(readPage);
        assertEqual(readPage!.pageId, 0);
        assertEqual(readPage!.records.length, 1);
        assertEqual(readPage!.records[0]!.key, "test-key");
    });

    suite.test("LightFileManager - 写入和加载快照", async () => {
        await fileManager.open();

        const pages = new Map<number, ReturnType<typeof fileManager.writePage extends (page: infer P) => unknown ? P : never>>();
        pages.set(0, {
            pageId: 0,
            records: [
                { key: "key1", value: Buffer.from("value1") },
                { key: "key2", value: Buffer.from("value2") },
            ],
            checksum: 0,
        });
        pages.set(1, {
            pageId: 1,
            records: [{ key: "key3", value: Buffer.from("value3") }],
            checksum: 0,
        });

        await fileManager.writeSnapshot(pages as any, 100n);

        const loadedRecords = await fileManager.loadAllRecords();
        assertEqual(loadedRecords.size, 3);
        assertTrue(loadedRecords.has("key1"));
        assertTrue(loadedRecords.has("key2"));
        assertTrue(loadedRecords.has("key3"));
    });

    suite.test("PageManager - 创建页面", () => {
        const pageManager = new PageManager(DEFAULT_PAGE_SIZE);

        const page = pageManager.createPage();
        assertEqual(page.pageId, 0);
        assertEqual(page.records.length, 0);

        const page2 = pageManager.createPage();
        assertEqual(page2.pageId, 1);
    });

    suite.test("PageManager - 获取和标记脏页", () => {
        const pageManager = new PageManager(DEFAULT_PAGE_SIZE);

        const page = pageManager.createPage();
        pageManager.markDirty(page.pageId);

        const dirtyPages = pageManager.getDirtyPages();
        assertEqual(dirtyPages.length, 1);
        assertEqual(dirtyPages[0]!.pageId, 0);

        pageManager.clearDirtyFlags();
        assertEqual(pageManager.getDirtyPages().length, 0);
    });

    suite.test("PageManager - 计算记录大小", () => {
        const pageManager = new PageManager(DEFAULT_PAGE_SIZE);

        const record = {
            key: "test-key",
            value: Buffer.from("test-value"),
        };

        const size = pageManager.calculateRecordSize(record);
        assertGreater(size, 0, "Record size should be greater than 0");
    });

    return suite;
}

/**
 * WALFile 测试套件
 */
function createWALFileTests(): TestSuite {
    const suite = describe("WALFile Tests");

    let tempDir: string;
    let walManager: WalManager;

    suite.beforeEach(async () => {
        tempDir = createTempDir("walfile-");
        walManager = new WalManager(tempDir, "test");
        await walManager.initialize();
    });

    suite.afterEach(async () => {
        if (walManager) {
            await walManager.close();
        }
        removeTempDir(tempDir);
    });

    suite.test("WalRecordSerializer - 序列化和反序列化插入记录", () => {
        const serializer = new WalRecordSerializer();

        const record = {
            lsn: 1n,
            transactionId: 1n,
            operationType: OperationType.Insert,
            key: "test-key",
            value: Buffer.from("test-value"),
            checksum: 0,
        };

        const buffer = serializer.serialize(record);
        assertTrue(buffer.length > 0);

        const result = serializer.deserialize(buffer);
        assertNotNull(result);
        assertEqual(result!.record.lsn, 1n);
        assertEqual(result!.record.key, "test-key");
        assertEqual(result!.record.value!.toString(), "test-value");
    });

    suite.test("WalRecordSerializer - 序列化和反序列化删除记录", () => {
        const serializer = new WalRecordSerializer();

        const record = {
            lsn: 2n,
            transactionId: 1n,
            operationType: OperationType.Delete,
            key: "test-key",
            value: null,
            checksum: 0,
        };

        const buffer = serializer.serialize(record);
        const result = serializer.deserialize(buffer);

        assertNotNull(result);
        assertEqual(result!.record.operationType, OperationType.Delete);
        assertNull(result!.record.value);
    });

    suite.test("WalManager - 写入插入记录", async () => {
        const result = await walManager.writeInsert("key1", Buffer.from("value1"));

        assertTrue(result.success);
        assertEqual(result.lsn, 1n);
        assertGreater(result.bytesWritten, 0);
    });

    suite.test("WalManager - 写入更新记录", async () => {
        await walManager.writeInsert("key1", Buffer.from("value1"));
        const result = await walManager.writeUpdate("key1", Buffer.from("value2"));

        assertTrue(result.success);
        assertEqual(result.lsn, 2n);
    });

    suite.test("WalManager - 写入删除记录", async () => {
        await walManager.writeInsert("key1", Buffer.from("value1"));
        const result = await walManager.writeDelete("key1");

        assertTrue(result.success);
        assertEqual(result.lsn, 2n);
    });

    suite.test("WalManager - 写入检查点标记", async () => {
        const result = await walManager.writeCheckpoint();

        assertTrue(result.success);
    });

    suite.test("WalManager - LSN 分配", async () => {
        const lsn1 = walManager.allocateLsn();
        const lsn2 = walManager.allocateLsn();
        const lsn3 = walManager.allocateLsn();

        assertEqual(lsn1, 1n);
        assertEqual(lsn2, 2n);
        assertEqual(lsn3, 3n);
    });

    suite.test("WalManager - 事务ID 分配", () => {
        const txId1 = walManager.allocateTransactionId();
        const txId2 = walManager.allocateTransactionId();

        assertEqual(txId1, 0n);
        assertEqual(txId2, 1n);
    });

    suite.test("WalManager - 读取所有记录", async () => {
        await walManager.writeInsert("key1", Buffer.from("value1"));
        await walManager.writeUpdate("key1", Buffer.from("value2"));
        await walManager.writeDelete("key2");

        const records = await walManager.readAllRecords();
        assertEqual(records.length, 3);
        assertEqual(records[0]!.operationType, OperationType.Insert);
        assertEqual(records[1]!.operationType, OperationType.Update);
        assertEqual(records[2]!.operationType, OperationType.Delete);
    });

    suite.test("WalManager - 清空 WAL", async () => {
        await walManager.writeInsert("key1", Buffer.from("value1"));
        await walManager.clear();

        const records = await walManager.readAllRecords();
        assertEqual(records.length, 0);
    });

    suite.test("WalReader - 迭代读取", async () => {
        await walManager.writeInsert("key1", Buffer.from("value1"));
        await walManager.writeInsert("key2", Buffer.from("value2"));
        await walManager.close();

        const reader = new WalReader(tempDir, "test");
        const records: any[] = [];

        for await (const record of reader.iterate()) {
            records.push(record);
        }

        assertEqual(records.length, 2);
    });

    return suite;
}

/**
 * SHMFile 测试套件
 */
function createSHMFileTests(): TestSuite {
    const suite = describe("SHMFile Tests");

    let tempDir: string;

    suite.beforeEach(() => {
        tempDir = createTempDir("shmfile-");
    });

    suite.afterEach(() => {
        removeTempDir(tempDir);
    });

    suite.test("ShmSerializer - 序列化和反序列化头部", () => {
        const serializer = new ShmSerializer();

        const header = {
            walFileSize: 1024n,
            lastCompleteLsn: 100n,
            recordCount: 10,
            checksum: 0,
        };

        const buffer = serializer.serializeHeader(header);
        const deserialized = serializer.deserializeHeader(buffer);

        assertEqual(deserialized.walFileSize, 1024n);
        assertEqual(deserialized.lastCompleteLsn, 100n);
        assertEqual(deserialized.recordCount, 10);
    });

    suite.test("ShmSerializer - 序列化和反序列化条目", () => {
        const serializer = new ShmSerializer();

        const entry = {
            pageId: 1,
            lsn: 50n,
            walOffset: 256n,
        };

        const buffer = serializer.serializeEntry(entry);
        const deserialized = serializer.deserializeEntry(buffer);

        assertEqual(deserialized.pageId, 1);
        assertEqual(deserialized.lsn, 50n);
        assertEqual(deserialized.walOffset, 256n);
    });

    suite.test("ShmHashTable - 插入和获取条目", () => {
        const hashTable = new ShmHashTable();

        const entry = {
            pageId: 1,
            lsn: 100n,
            walOffset: 512n,
        };

        hashTable.set(entry);

        const retrieved = hashTable.get(1);
        assertNotNull(retrieved);
        assertEqual(retrieved!.lsn, 100n);
    });

    suite.test("ShmHashTable - 更新条目（更高 LSN）", () => {
        const hashTable = new ShmHashTable();

        hashTable.set({ pageId: 1, lsn: 100n, walOffset: 512n });
        hashTable.set({ pageId: 1, lsn: 200n, walOffset: 1024n });

        const retrieved = hashTable.get(1);
        assertNotNull(retrieved);
        assertEqual(retrieved!.lsn, 200n);
        assertEqual(retrieved!.walOffset, 1024n);
    });

    suite.test("ShmHashTable - 不更新条目（更低 LSN）", () => {
        const hashTable = new ShmHashTable();

        hashTable.set({ pageId: 1, lsn: 200n, walOffset: 1024n });
        hashTable.set({ pageId: 1, lsn: 100n, walOffset: 512n });

        const retrieved = hashTable.get(1);
        assertNotNull(retrieved);
        assertEqual(retrieved!.lsn, 200n);
    });

    suite.test("ShmHashTable - 删除条目", () => {
        const hashTable = new ShmHashTable();

        hashTable.set({ pageId: 1, lsn: 100n, walOffset: 512n });
        assertTrue(hashTable.delete(1));
        assertFalse(hashTable.has(1));
    });

    suite.test("ShmFileManager - 保存和加载", async () => {
        const manager = new ShmFileManager(tempDir, "test");

        manager.updateEntry(1, 100n, 512n);
        manager.updateEntry(2, 200n, 1024n);
        manager.updateHeader(2048n, 200n);

        await manager.save();

        const manager2 = new ShmFileManager(tempDir, "test");
        const loaded = await manager2.load();

        assertTrue(loaded);
        assertEqual(manager2.getEntryCount(), 2);
        assertEqual(manager2.getLastCompleteLsn(), 200n);
    });

    suite.test("WalIndexManager - 初始化", async () => {
        const manager = new WalIndexManager(tempDir, "test");
        await manager.initialize();

        const dirtyPages = manager.getDirtyPages();
        assertEqual(dirtyPages.length, 0);

        await manager.close();
    });

    suite.test("WalIndexManager - 记录写入和获取", async () => {
        const manager = new WalIndexManager(tempDir, "test");
        await manager.initialize();

        await manager.recordWrite(1, 100n, 512n);
        await manager.recordWrite(2, 200n, 1024n);

        const entry1 = manager.getPageLocation(1);
        assertNotNull(entry1);
        assertEqual(entry1!.lsn, 100n);

        const entry2 = manager.getPageLocation(2);
        assertNotNull(entry2);
        assertEqual(entry2!.lsn, 200n);

        await manager.close();
    });

    return suite;
}

/**
 * Checkpoint 测试套件
 */
function createCheckpointTests(): TestSuite {
    const suite = describe("Checkpoint Tests");

    let tempDir: string;
    let storageEngine: StorageEngine;

    suite.beforeEach(async () => {
        tempDir = createTempDir("checkpoint-");
        storageEngine = new StorageEngine(tempDir, "test", {
            autoCheckpoint: false,
        });
        await storageEngine.initialize();
    });

    suite.afterEach(async () => {
        if (storageEngine) {
            await storageEngine.close();
        }
        removeTempDir(tempDir);
    });

    suite.test("执行检查点", async () => {
        await storageEngine.insert("key1", Buffer.from("value1"));
        await storageEngine.insert("key2", Buffer.from("value2"));

        const result = await storageEngine.checkpoint();

        assertTrue(result.timestamp > 0);
        assertGreater(result.lsn, 0n);
        assertGreater(result.pagesWritten, 0);
        assertGreater(result.bytesWritten, 0);
    });

    suite.test("检查点后数据持久化", async () => {
        await storageEngine.insert("key1", Buffer.from("value1"));
        await storageEngine.insert("key2", Buffer.from("value2"));
        await storageEngine.checkpoint();
        await storageEngine.close();

        const engine2 = new StorageEngine(tempDir, "test", { autoCheckpoint: false });
        await engine2.initialize();

        const value1 = await engine2.get("key1");
        const value2 = await engine2.get("key2");

        assertNotNull(value1);
        assertNotNull(value2);
        assertEqual(value1!.toString(), "value1");
        assertEqual(value2!.toString(), "value2");

        await engine2.close();
    });

    suite.test("检查点后 WAL 清空", async () => {
        await storageEngine.insert("key1", Buffer.from("value1"));
        await storageEngine.checkpoint();

        const stats = await storageEngine.getStats();
        assertEqual(stats.walFileSize, 0);
    });

    suite.test("多次检查点", async () => {
        await storageEngine.insert("key1", Buffer.from("value1"));
        await storageEngine.checkpoint();

        await storageEngine.insert("key2", Buffer.from("value2"));
        await storageEngine.checkpoint();

        await storageEngine.insert("key3", Buffer.from("value3"));
        await storageEngine.checkpoint();

        const keys = await storageEngine.getAllKeys();
        assertEqual(keys.length, 3);
    });

    return suite;
}

/**
 * Recovery 测试套件
 */
function createRecoveryTests(): TestSuite {
    const suite = describe("Recovery Tests");

    let tempDir: string;

    suite.beforeEach(() => {
        tempDir = createTempDir("recovery-");
    });

    suite.afterEach(() => {
        removeTempDir(tempDir);
    });

    suite.test("空数据库恢复", async () => {
        const engine = new StorageEngine(tempDir, "test", { autoCheckpoint: false });
        const result = await engine.initialize();

        assertTrue(result.success);
        assertEqual(result.recordsReplayed, 0);

        await engine.close();
    });

    suite.test("从 WAL 恢复", async () => {
        const engine1 = new StorageEngine(tempDir, "test", { autoCheckpoint: false });
        await engine1.initialize();

        await engine1.insert("key1", Buffer.from("value1"));
        await engine1.insert("key2", Buffer.from("value2"));
        await engine1.insert("key3", Buffer.from("value3"));

        await engine1.sync();
        await engine1.close();

        const engine2 = new StorageEngine(tempDir, "test", { autoCheckpoint: false });
        const result = await engine2.initialize();

        assertTrue(result.success);
        assertEqual(result.recordsReplayed, 3);

        const keys = await engine2.getAllKeys();
        assertEqual(keys.length, 3);

        await engine2.close();
    });

    suite.test("从检查点恢复", async () => {
        const engine1 = new StorageEngine(tempDir, "test", { autoCheckpoint: false });
        await engine1.initialize();

        await engine1.insert("key1", Buffer.from("value1"));
        await engine1.insert("key2", Buffer.from("value2"));
        await engine1.checkpoint();

        await engine1.insert("key3", Buffer.from("value3"));

        await engine1.sync();
        await engine1.close();

        const engine2 = new StorageEngine(tempDir, "test", { autoCheckpoint: false });
        const result = await engine2.initialize();

        assertTrue(result.success);

        const keys = await engine2.getAllKeys();
        assertEqual(keys.length, 3);

        const value1 = await engine2.get("key1");
        const value3 = await engine2.get("key3");
        assertNotNull(value1);
        assertNotNull(value3);

        await engine2.close();
    });

    suite.test("恢复时应用删除操作", async () => {
        const engine1 = new StorageEngine(tempDir, "test", { autoCheckpoint: false });
        await engine1.initialize();

        await engine1.insert("key1", Buffer.from("value1"));
        await engine1.insert("key2", Buffer.from("value2"));
        await engine1.delete("key1");

        await engine1.sync();
        await engine1.close();

        const engine2 = new StorageEngine(tempDir, "test", { autoCheckpoint: false });
        await engine2.initialize();

        const keys = await engine2.getAllKeys();
        assertEqual(keys.length, 1);
        assertFalse(keys.includes("key1"));
        assertTrue(keys.includes("key2"));

        await engine2.close();
    });

    suite.test("恢复时应用更新操作", async () => {
        const engine1 = new StorageEngine(tempDir, "test", { autoCheckpoint: false });
        await engine1.initialize();

        await engine1.insert("key1", Buffer.from("value1"));
        await engine1.update("key1", Buffer.from("value2"));
        await engine1.update("key1", Buffer.from("value3"));

        await engine1.sync();
        await engine1.close();

        const engine2 = new StorageEngine(tempDir, "test", { autoCheckpoint: false });
        await engine2.initialize();

        const value = await engine2.get("key1");
        assertNotNull(value);
        assertEqual(value!.toString(), "value3");

        await engine2.close();
    });

    suite.test("RecoveryValidator - 验证记录完整性", () => {
        const validator = new RecoveryValidator();

        const records = new Map<string, Buffer>();
        records.set("key1", Buffer.from("value1"));
        records.set("key2", Buffer.from("value2"));

        const result = validator.validateRecords(records);
        assertTrue(result.valid);
        assertEqual(result.errors.length, 0);
    });

    suite.test("RecoveryValidator - 检测无效记录", () => {
        const validator = new RecoveryValidator();

        const records = new Map<string, Buffer>();
        records.set("", Buffer.from("value1"));
        records.set("key2", Buffer.alloc(0));

        const result = validator.validateRecords(records);
        assertFalse(result.valid);
        assertGreater(result.errors.length, 0);
    });

    return suite;
}

/**
 * StorageEngine 综合测试套件
 */
function createStorageEngineTests(): TestSuite {
    const suite = describe("StorageEngine Tests");

    let tempDir: string;
    let engine: StorageEngine;

    suite.beforeEach(async () => {
        tempDir = createTempDir("storage-");
        engine = new StorageEngine(tempDir, "test", { autoCheckpoint: false });
        await engine.initialize();
    });

    suite.afterEach(async () => {
        if (engine) {
            await engine.close();
        }
        removeTempDir(tempDir);
    });

    suite.test("初始化状态", () => {
        assertTrue(engine.isInitialized());
        assertFalse(engine.isClosed());
    });

    suite.test("插入记录", async () => {
        const result = await engine.insert("key1", Buffer.from("value1"));

        assertTrue(result.success);
        assertEqual(result.lsn, 1n);
        assertGreater(result.bytesWritten, 0);
    });

    suite.test("重复插入失败", async () => {
        await engine.insert("key1", Buffer.from("value1"));

        await assertThrows(() => engine.insert("key1", Buffer.from("value2")), StorageError);
    });

    suite.test("读取记录", async () => {
        await engine.insert("key1", Buffer.from("value1"));

        const value = await engine.get("key1");
        assertNotNull(value);
        assertEqual(value!.toString(), "value1");
    });

    suite.test("读取不存在的记录", async () => {
        const value = await engine.get("nonexistent");
        assertNull(value);
    });

    suite.test("更新记录", async () => {
        await engine.insert("key1", Buffer.from("value1"));
        const result = await engine.update("key1", Buffer.from("value2"));

        assertTrue(result.success);

        const value = await engine.get("key1");
        assertEqual(value!.toString(), "value2");
    });

    suite.test("更新不存在的记录失败", async () => {
        await assertThrows(() => engine.update("nonexistent", Buffer.from("value")), StorageError);
    });

    suite.test("Upsert 插入新记录", async () => {
        const result = await engine.upsert("key1", Buffer.from("value1"));

        assertTrue(result.success);

        const value = await engine.get("key1");
        assertNotNull(value);
    });

    suite.test("Upsert 更新现有记录", async () => {
        await engine.insert("key1", Buffer.from("value1"));
        await engine.upsert("key1", Buffer.from("value2"));

        const value = await engine.get("key1");
        assertEqual(value!.toString(), "value2");
    });

    suite.test("删除记录", async () => {
        await engine.insert("key1", Buffer.from("value1"));
        const result = await engine.delete("key1");

        assertTrue(result.success);

        const value = await engine.get("key1");
        assertNull(value);
    });

    suite.test("删除不存在的记录失败", async () => {
        await assertThrows(() => engine.delete("nonexistent"), StorageError);
    });

    suite.test("检查记录是否存在", async () => {
        await engine.insert("key1", Buffer.from("value1"));

        assertTrue(await engine.exists("key1"));
        assertFalse(await engine.exists("nonexistent"));
    });

    suite.test("获取所有键", async () => {
        await engine.insert("key1", Buffer.from("value1"));
        await engine.insert("key2", Buffer.from("value2"));
        await engine.insert("key3", Buffer.from("value3"));

        const keys = await engine.getAllKeys();
        assertEqual(keys.length, 3);
        assertTrue(keys.includes("key1"));
        assertTrue(keys.includes("key2"));
        assertTrue(keys.includes("key3"));
    });

    suite.test("获取记录数量", async () => {
        await engine.insert("key1", Buffer.from("value1"));
        await engine.insert("key2", Buffer.from("value2"));

        const count = await engine.getRecordCount();
        assertEqual(count, 2);
    });

    suite.test("批量插入", async () => {
        const entries = [
            { key: "key1", value: Buffer.from("value1") },
            { key: "key2", value: Buffer.from("value2") },
            { key: "key3", value: Buffer.from("value3") },
        ];

        const result = await engine.insertBatch(entries);

        assertTrue(result.success);
        assertEqual(result.recordsWritten, 3);
        assertEqual(await engine.getRecordCount(), 3);
    });

    suite.test("获取统计信息", async () => {
        await engine.insert("key1", Buffer.from("value1"));

        const stats = await engine.getStats();

        assertEqual(stats.recordCount, 1);
        assertGreater(stats.currentLsn, 0n);
        assertGreater(stats.walFileSize, 0);
    });

    suite.test("关闭引擎", async () => {
        await engine.close();

        assertFalse(engine.isInitialized());
        assertTrue(engine.isClosed());
    });

    suite.test("关闭后操作失败", async () => {
        await engine.close();

        await assertThrows(() => engine.insert("key1", Buffer.from("value1")), StorageError);
    });

    return suite;
}

/**
 * 导出所有存储引擎测试
 */
export function getStorageEngineTests(): TestSuite[] {
    return [
        createLightFileTests(),
        createWALFileTests(),
        createSHMFileTests(),
        createCheckpointTests(),
        createRecoveryTests(),
        createStorageEngineTests(),
    ];
}
