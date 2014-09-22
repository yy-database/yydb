/**
 * 索引引擎单元测试
 * 测试 B+ 树主键索引、二级索引、索引管理器和范围查询
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
    assertLength,
    assertContains,
    assertNotContains,
    TestSuite,
    randomString,
    randomInt,
} from "./test-utils";
import { BPlusTree } from "../src/indexes/BPlusTree";
import { PrimaryIndex } from "../src/indexes/PrimaryIndex";
import { SecondaryIndex } from "../src/indexes/SecondaryIndex";
import { IndexManager, CollectionIndexConfig } from "../src/indexes/IndexManager";
import { IndexType } from "../src/indexes/types";
import { DuplicateKeyError, KeyNotFoundError, IndexNotFoundError, IndexExistsError } from "../src/indexes/errors";

/**
 * B+ 树测试套件
 */
function createBPlusTreeTests(): TestSuite {
    const suite = describe("BPlusTree Tests");

    suite.test("创建空树", () => {
        const tree = new BPlusTree<string, number>();

        assertEqual(tree.Size, 0);
        assertTrue(tree.isEmpty());
    });

    suite.test("插入单个键值对", () => {
        const tree = new BPlusTree<string, number>();

        const result = tree.insert("key1", 100);
        assertNull(result);

        assertEqual(tree.Size, 1);
        assertFalse(tree.isEmpty());
    });

    suite.test("插入重复键返回旧值", () => {
        const tree = new BPlusTree<string, number>();

        tree.insert("key1", 100);
        const result = tree.insert("key1", 200);

        assertEqual(result, 100);
        assertEqual(tree.Size, 1);
    });

    suite.test("查找存在的键", () => {
        const tree = new BPlusTree<string, number>();

        tree.insert("key1", 100);
        tree.insert("key2", 200);

        assertEqual(tree.find("key1"), 100);
        assertEqual(tree.find("key2"), 200);
    });

    suite.test("查找不存在的键", () => {
        const tree = new BPlusTree<string, number>();

        tree.insert("key1", 100);

        assertNull(tree.find("nonexistent"));
    });

    suite.test("检查键是否存在", () => {
        const tree = new BPlusTree<string, number>();

        tree.insert("key1", 100);

        assertTrue(tree.has("key1"));
        assertFalse(tree.has("nonexistent"));
    });

    suite.test("删除存在的键", () => {
        const tree = new BPlusTree<string, number>();

        tree.insert("key1", 100);
        const result = tree.delete("key1");

        assertEqual(result, 100);
        assertEqual(tree.Size, 0);
        assertFalse(tree.has("key1"));
    });

    suite.test("删除不存在的键", () => {
        const tree = new BPlusTree<string, number>();

        const result = tree.delete("nonexistent");

        assertNull(result);
    });

    suite.test("更新存在的键", () => {
        const tree = new BPlusTree<string, number>();

        tree.insert("key1", 100);
        const result = tree.update("key1", 200);

        assertTrue(result);
        assertEqual(tree.find("key1"), 200);
    });

    suite.test("更新不存在的键", () => {
        const tree = new BPlusTree<string, number>();

        const result = tree.update("nonexistent", 100);

        assertFalse(result);
    });

    suite.test("范围查询 - 全部", () => {
        const tree = new BPlusTree<number, string>();

        tree.insert(1, "one");
        tree.insert(2, "two");
        tree.insert(3, "three");
        tree.insert(4, "four");
        tree.insert(5, "five");

        const result = tree.range();

        assertEqual(result.length, 5);
    });

    suite.test("范围查询 - 指定范围", () => {
        const tree = new BPlusTree<number, string>();

        tree.insert(1, "one");
        tree.insert(2, "two");
        tree.insert(3, "three");
        tree.insert(4, "four");
        tree.insert(5, "five");

        const result = tree.range(2, 4);

        assertEqual(result.length, 3);
        assertEqual(result[0]!.key, 2);
        assertEqual(result[1]!.key, 3);
        assertEqual(result[2]!.key, 4);
    });

    suite.test("范围查询 - 排除边界", () => {
        const tree = new BPlusTree<number, string>();

        tree.insert(1, "one");
        tree.insert(2, "two");
        tree.insert(3, "three");
        tree.insert(4, "four");
        tree.insert(5, "five");

        const result = tree.range(2, 4, false, false);

        assertEqual(result.length, 1);
        assertEqual(result[0]!.key, 3);
    });

    suite.test("获取所有键", () => {
        const tree = new BPlusTree<number, string>();

        tree.insert(3, "three");
        tree.insert(1, "one");
        tree.insert(2, "two");

        const keys = tree.keys();

        assertEqual(keys.length, 3);
        assertEqual(keys[0], 1);
        assertEqual(keys[1], 2);
        assertEqual(keys[2], 3);
    });

    suite.test("获取所有值", () => {
        const tree = new BPlusTree<number, string>();

        tree.insert(3, "three");
        tree.insert(1, "one");
        tree.insert(2, "two");

        const values = tree.values();

        assertEqual(values.length, 3);
        assertEqual(values[0], "one");
        assertEqual(values[1], "two");
        assertEqual(values[2], "three");
    });

    suite.test("获取最小键", () => {
        const tree = new BPlusTree<number, string>();

        tree.insert(5, "five");
        tree.insert(2, "two");
        tree.insert(8, "eight");

        assertEqual(tree.minKey(), 2);
    });

    suite.test("获取最大键", () => {
        const tree = new BPlusTree<number, string>();

        tree.insert(5, "five");
        tree.insert(2, "two");
        tree.insert(8, "eight");

        assertEqual(tree.maxKey(), 8);
    });

    suite.test("获取第一个键值对", () => {
        const tree = new BPlusTree<number, string>();

        tree.insert(2, "two");
        tree.insert(1, "one");

        const first = tree.first();

        assertNotNull(first);
        assertEqual(first!.key, 1);
        assertEqual(first!.value, "one");
    });

    suite.test("获取最后一个键值对", () => {
        const tree = new BPlusTree<number, string>();

        tree.insert(2, "two");
        tree.insert(1, "one");

        const last = tree.last();

        assertNotNull(last);
        assertEqual(last!.key, 2);
        assertEqual(last!.value, "two");
    });

    suite.test("清空树", () => {
        const tree = new BPlusTree<number, string>();

        tree.insert(1, "one");
        tree.insert(2, "two");
        tree.clear();

        assertEqual(tree.Size, 0);
        assertTrue(tree.isEmpty());
    });

    suite.test("迭代器", () => {
        const tree = new BPlusTree<number, string>();

        tree.insert(1, "one");
        tree.insert(2, "two");
        tree.insert(3, "three");

        const entries: any[] = [];
        for (const entry of tree) {
            entries.push(entry);
        }

        assertEqual(entries.length, 3);
    });

    suite.test("大量插入", () => {
        const tree = new BPlusTree<number, number>();
        const count = 1000;

        for (let i = 0; i < count; i++) {
            tree.insert(i, i * 10);
        }

        assertEqual(tree.Size, count);

        for (let i = 0; i < count; i++) {
            assertEqual(tree.find(i), i * 10);
        }
    });

    suite.test("大量删除", () => {
        const tree = new BPlusTree<number, number>();
        const count = 100;

        for (let i = 0; i < count; i++) {
            tree.insert(i, i * 10);
        }

        for (let i = 0; i < count; i += 2) {
            tree.delete(i);
        }

        assertEqual(tree.Size, count / 2);

        for (let i = 0; i < count; i++) {
            if (i % 2 === 0) {
                assertFalse(tree.has(i));
            } else {
                assertTrue(tree.has(i));
            }
        }
    });

    suite.test("自定义阶数", () => {
        const tree = new BPlusTree<number, string>(8);

        assertEqual(tree.Size, 0);
    });

    suite.test("阶数太小抛出错误", () => {
        assertThrows(
            () => new BPlusTree<number, string>(2),
            Error,
        );
    });

    suite.test("字符串键排序", () => {
        const tree = new BPlusTree<string, number>();

        tree.insert("banana", 2);
        tree.insert("apple", 1);
        tree.insert("cherry", 3);

        const keys = tree.keys();

        assertEqual(keys[0], "apple");
        assertEqual(keys[1], "banana");
        assertEqual(keys[2], "cherry");
    });

    return suite;
}

/**
 * 主键索引测试套件
 */
function createPrimaryIndexTests(): TestSuite {
    const suite = describe("PrimaryIndex Tests");

    suite.test("创建主键索引", () => {
        const index = new PrimaryIndex<{ id: number; name: string }>({
            name: "test_pk",
            fieldName: "id",
        });

        assertEqual(index.Name, "test_pk");
        assertEqual(index.FieldName, "id");
        assertEqual(index.Type, IndexType.Primary);
        assertEqual(index.Size, 0);
        assertTrue(index.isEmpty());
    });

    suite.test("插入记录", () => {
        const index = new PrimaryIndex<{ id: number; name: string }>({
            name: "test_pk",
            fieldName: "id",
        });

        const doc = { id: 1, name: "test" };
        const result = index.insert(1, doc);

        assertTrue(result);
        assertEqual(index.Size, 1);
    });

    suite.test("插入重复键", () => {
        const index = new PrimaryIndex<{ id: number; name: string }>({
            name: "test_pk",
            fieldName: "id",
        });

        index.insert(1, { id: 1, name: "test1" });
        const result = index.insert(1, { id: 1, name: "test2" });

        assertFalse(result);
    });

    suite.test("插入重复键抛出异常", () => {
        const index = new PrimaryIndex<{ id: number; name: string }>({
            name: "test_pk",
            fieldName: "id",
        });

        index.insert(1, { id: 1, name: "test1" });

        assertThrows(
            () => index.insert(1, { id: 1, name: "test2" }, true),
            DuplicateKeyError,
        );
    });

    suite.test("查找记录", () => {
        const index = new PrimaryIndex<{ id: number; name: string }>({
            name: "test_pk",
            fieldName: "id",
        });

        const doc = { id: 1, name: "test" };
        index.insert(1, doc);

        const found = index.find(1);

        assertNotNull(found);
        assertEqual(found!.id, 1);
        assertEqual(found!.name, "test");
    });

    suite.test("查找不存在的记录", () => {
        const index = new PrimaryIndex<{ id: number; name: string }>({
            name: "test_pk",
            fieldName: "id",
        });

        const found = index.find(999);

        assertNull(found);
    });

    suite.test("检查键是否存在", () => {
        const index = new PrimaryIndex<{ id: number; name: string }>({
            name: "test_pk",
            fieldName: "id",
        });

        index.insert(1, { id: 1, name: "test" });

        assertTrue(index.has(1));
        assertFalse(index.has(999));
    });

    suite.test("更新记录", () => {
        const index = new PrimaryIndex<{ id: number; name: string }>({
            name: "test_pk",
            fieldName: "id",
        });

        index.insert(1, { id: 1, name: "test1" });
        const result = index.update(1, { id: 1, name: "test2" });

        assertTrue(result);
        assertEqual(index.find(1)!.name, "test2");
    });

    suite.test("更新不存在的记录", () => {
        const index = new PrimaryIndex<{ id: number; name: string }>({
            name: "test_pk",
            fieldName: "id",
        });

        const result = index.update(999, { id: 999, name: "test" });

        assertFalse(result);
    });

    suite.test("更新不存在的记录抛出异常", () => {
        const index = new PrimaryIndex<{ id: number; name: string }>({
            name: "test_pk",
            fieldName: "id",
        });

        assertThrows(
            () => index.update(999, { id: 999, name: "test" }, true),
            KeyNotFoundError,
        );
    });

    suite.test("Upsert 插入新记录", () => {
        const index = new PrimaryIndex<{ id: number; name: string }>({
            name: "test_pk",
            fieldName: "id",
        });

        index.upsert(1, { id: 1, name: "test" });

        assertEqual(index.Size, 1);
        assertTrue(index.has(1));
    });

    suite.test("Upsert 更新现有记录", () => {
        const index = new PrimaryIndex<{ id: number; name: string }>({
            name: "test_pk",
            fieldName: "id",
        });

        index.insert(1, { id: 1, name: "test1" });
        index.upsert(1, { id: 1, name: "test2" });

        assertEqual(index.find(1)!.name, "test2");
        assertEqual(index.Size, 1);
    });

    suite.test("删除记录", () => {
        const index = new PrimaryIndex<{ id: number; name: string }>({
            name: "test_pk",
            fieldName: "id",
        });

        index.insert(1, { id: 1, name: "test" });
        const deleted = index.delete(1);

        assertNotNull(deleted);
        assertEqual(deleted!.name, "test");
        assertEqual(index.Size, 0);
    });

    suite.test("删除不存在的记录", () => {
        const index = new PrimaryIndex<{ id: number; name: string }>({
            name: "test_pk",
            fieldName: "id",
        });

        const deleted = index.delete(999);

        assertNull(deleted);
    });

    suite.test("范围查询", () => {
        const index = new PrimaryIndex<{ id: number; name: string }>({
            name: "test_pk",
            fieldName: "id",
        });

        for (let i = 1; i <= 5; i++) {
            index.insert(i, { id: i, name: `test${i}` });
        }

        const result = index.range(2, 4);

        assertEqual(result.length, 3);
        assertEqual(result[0]!.key, 2);
        assertEqual(result[1]!.key, 3);
        assertEqual(result[2]!.key, 4);
    });

    suite.test("大于查询", () => {
        const index = new PrimaryIndex<{ id: number; name: string }>({
            name: "test_pk",
            fieldName: "id",
        });

        for (let i = 1; i <= 5; i++) {
            index.insert(i, { id: i, name: `test${i}` });
        }

        const result = index.greaterThan(3);

        assertEqual(result.length, 2);
        assertEqual(result[0]!.key, 4);
        assertEqual(result[1]!.key, 5);
    });

    suite.test("小于查询", () => {
        const index = new PrimaryIndex<{ id: number; name: string }>({
            name: "test_pk",
            fieldName: "id",
        });

        for (let i = 1; i <= 5; i++) {
            index.insert(i, { id: i, name: `test${i}` });
        }

        const result = index.lessThan(3);

        assertEqual(result.length, 2);
        assertEqual(result[0]!.key, 1);
        assertEqual(result[1]!.key, 2);
    });

    suite.test("批量插入", () => {
        const index = new PrimaryIndex<{ id: number; name: string }>({
            name: "test_pk",
            fieldName: "id",
        });

        const entries = [
            { key: 1, value: { id: 1, name: "test1" } },
            { key: 2, value: { id: 2, name: "test2" } },
            { key: 3, value: { id: 3, name: "test3" } },
        ];

        const result = index.insertBatch(entries);

        assertEqual(result.success, 3);
        assertEqual(result.failed, 0);
        assertEqual(index.Size, 3);
    });

    suite.test("批量删除", () => {
        const index = new PrimaryIndex<{ id: number; name: string }>({
            name: "test_pk",
            fieldName: "id",
        });

        for (let i = 1; i <= 5; i++) {
            index.insert(i, { id: i, name: `test${i}` });
        }

        const result = index.deleteBatch([1, 3, 5]);

        assertEqual(result.success, 3);
        assertEqual(result.failed, 0);
        assertEqual(index.Size, 2);
    });

    suite.test("获取统计信息", () => {
        const index = new PrimaryIndex<{ id: number; name: string }>({
            name: "test_pk",
            fieldName: "id",
        });

        index.insert(1, { id: 1, name: "test" });
        const stats = index.getStats();

        assertEqual(stats.name, "test_pk");
        assertEqual(stats.type, IndexType.Primary);
        assertEqual(stats.fieldName, "id");
        assertEqual(stats.keyCount, 1);
        assertTrue(stats.unique);
    });

    suite.test("验证索引一致性", () => {
        const index = new PrimaryIndex<{ id: number; name: string }>({
            name: "test_pk",
            fieldName: "id",
        });

        for (let i = 1; i <= 10; i++) {
            index.insert(i, { id: i, name: `test${i}` });
        }

        const result = index.validate();

        assertTrue(result.valid);
        assertEqual(result.errors.length, 0);
    });

    suite.test("清空索引", () => {
        const index = new PrimaryIndex<{ id: number; name: string }>({
            name: "test_pk",
            fieldName: "id",
        });

        index.insert(1, { id: 1, name: "test" });
        index.clear();

        assertEqual(index.Size, 0);
        assertTrue(index.isEmpty());
    });

    return suite;
}

/**
 * 二级索引测试套件
 */
function createSecondaryIndexTests(): TestSuite {
    const suite = describe("SecondaryIndex Tests");

    suite.test("创建二级索引", () => {
        const index = new SecondaryIndex({
            name: "name_idx",
            fieldName: "name",
        });

        assertEqual(index.Name, "name_idx");
        assertEqual(index.FieldName, "name");
        assertEqual(index.Type, IndexType.Secondary);
        assertFalse(index.IsUnique);
        assertFalse(index.IsSparse);
        assertEqual(index.Size, 0);
    });

    suite.test("创建唯一索引", () => {
        const index = new SecondaryIndex({
            name: "email_idx",
            fieldName: "email",
            unique: true,
        });

        assertTrue(index.IsUnique);
        assertEqual(index.Type, IndexType.Unique);
    });

    suite.test("创建稀疏索引", () => {
        const index = new SecondaryIndex({
            name: "optional_idx",
            fieldName: "optional",
            sparse: true,
        });

        assertTrue(index.IsSparse);
    });

    suite.test("插入索引条目", () => {
        const index = new SecondaryIndex({
            name: "name_idx",
            fieldName: "name",
        });

        const result = index.insert("Alice", 1);

        assertTrue(result);
        assertEqual(index.Size, 1);
    });

    suite.test("插入多个主键对应同一索引值", () => {
        const index = new SecondaryIndex({
            name: "name_idx",
            fieldName: "name",
        });

        index.insert("Alice", 1);
        index.insert("Alice", 2);
        index.insert("Alice", 3);

        assertEqual(index.Size, 3);
        assertEqual(index.ValueCount, 1);
    });

    suite.test("唯一索引拒绝重复值", () => {
        const index = new SecondaryIndex({
            name: "email_idx",
            fieldName: "email",
            unique: true,
        });

        index.insert("alice@example.com", 1);
        const result = index.insert("alice@example.com", 2);

        assertFalse(result);
    });

    suite.test("唯一索引重复值抛出异常", () => {
        const index = new SecondaryIndex({
            name: "email_idx",
            fieldName: "email",
            unique: true,
        });

        index.insert("alice@example.com", 1);

        assertThrows(
            () => index.insert("alice@example.com", 2, true),
            DuplicateKeyError,
        );
    });

    suite.test("稀疏索引忽略 null 值", () => {
        const index = new SecondaryIndex({
            name: "optional_idx",
            fieldName: "optional",
            sparse: true,
        });

        const result = index.insert(null, 1);

        assertTrue(result);
        assertEqual(index.Size, 0);
    });

    suite.test("稀疏索引忽略 undefined 值", () => {
        const index = new SecondaryIndex({
            name: "optional_idx",
            fieldName: "optional",
            sparse: true,
        });

        const result = index.insert(undefined, 1);

        assertTrue(result);
        assertEqual(index.Size, 0);
    });

    suite.test("等值查询", () => {
        const index = new SecondaryIndex({
            name: "name_idx",
            fieldName: "name",
        });

        index.insert("Alice", 1);
        index.insert("Alice", 2);
        index.insert("Bob", 3);

        const result = index.find("Alice");

        assertEqual(result.size, 2);
        assertTrue(result.has(1));
        assertTrue(result.has(2));
    });

    suite.test("查询不存在的值", () => {
        const index = new SecondaryIndex({
            name: "name_idx",
            fieldName: "name",
        });

        index.insert("Alice", 1);

        const result = index.find("Bob");

        assertEqual(result.size, 0);
    });

    suite.test("检查值是否存在", () => {
        const index = new SecondaryIndex({
            name: "name_idx",
            fieldName: "name",
        });

        index.insert("Alice", 1);

        assertTrue(index.has("Alice"));
        assertFalse(index.has("Bob"));
    });

    suite.test("检查条目是否存在", () => {
        const index = new SecondaryIndex({
            name: "name_idx",
            fieldName: "name",
        });

        index.insert("Alice", 1);

        assertTrue(index.hasEntry("Alice", 1));
        assertFalse(index.hasEntry("Alice", 2));
        assertFalse(index.hasEntry("Bob", 1));
    });

    suite.test("删除索引条目", () => {
        const index = new SecondaryIndex({
            name: "name_idx",
            fieldName: "name",
        });

        index.insert("Alice", 1);
        index.insert("Alice", 2);

        const result = index.delete("Alice", 1);

        assertTrue(result);
        assertEqual(index.Size, 1);
        assertFalse(index.hasEntry("Alice", 1));
        assertTrue(index.hasEntry("Alice", 2));
    });

    suite.test("删除不存在的条目", () => {
        const index = new SecondaryIndex({
            name: "name_idx",
            fieldName: "name",
        });

        const result = index.delete("Alice", 1);

        assertFalse(result);
    });

    suite.test("更新索引条目", () => {
        const index = new SecondaryIndex({
            name: "name_idx",
            fieldName: "name",
        });

        index.insert("Alice", 1);

        const result = index.update("Alice", "Bob", 1);

        assertTrue(result);
        assertFalse(index.has("Alice"));
        assertTrue(index.has("Bob"));
        assertTrue(index.hasEntry("Bob", 1));
    });

    suite.test("更新到相同值", () => {
        const index = new SecondaryIndex({
            name: "name_idx",
            fieldName: "name",
        });

        index.insert("Alice", 1);

        const result = index.update("Alice", "Alice", 1);

        assertTrue(result);
        assertTrue(index.hasEntry("Alice", 1));
    });

    suite.test("条件查询 - $eq", () => {
        const index = new SecondaryIndex({
            name: "name_idx",
            fieldName: "name",
        });

        index.insert("Alice", 1);
        index.insert("Bob", 2);
        index.insert("Alice", 3);

        const result = index.query({ $eq: "Alice" });

        assertEqual(result.size, 2);
    });

    suite.test("条件查询 - $in", () => {
        const index = new SecondaryIndex({
            name: "name_idx",
            fieldName: "name",
        });

        index.insert("Alice", 1);
        index.insert("Bob", 2);
        index.insert("Charlie", 3);

        const result = index.query({ $in: ["Alice", "Bob"] });

        assertEqual(result.size, 2);
        assertTrue(result.has(1));
        assertTrue(result.has(2));
    });

    suite.test("条件查询 - $ne", () => {
        const index = new SecondaryIndex({
            name: "name_idx",
            fieldName: "name",
        });

        index.insert("Alice", 1);
        index.insert("Bob", 2);
        index.insert("Charlie", 3);

        const result = index.query({ $ne: "Alice" });

        assertEqual(result.size, 2);
        assertFalse(result.has(1));
    });

    suite.test("范围查询", () => {
        const index = new SecondaryIndex({
            name: "age_idx",
            fieldName: "age",
        });

        index.insert(20, 1);
        index.insert(25, 2);
        index.insert(30, 3);
        index.insert(35, 4);
        index.insert(40, 5);

        const result = index.range(25, 35);

        assertEqual(result.size, 3);
        assertTrue(result.has(2));
        assertTrue(result.has(3));
        assertTrue(result.has(4));
    });

    suite.test("范围查询 - 排除边界", () => {
        const index = new SecondaryIndex({
            name: "age_idx",
            fieldName: "age",
        });

        index.insert(20, 1);
        index.insert(25, 2);
        index.insert(30, 3);
        index.insert(35, 4);
        index.insert(40, 5);

        const result = index.range(25, 35, false, false);

        assertEqual(result.size, 1);
        assertTrue(result.has(3));
    });

    suite.test("获取所有索引值", () => {
        const index = new SecondaryIndex({
            name: "name_idx",
            fieldName: "name",
        });

        index.insert("Alice", 1);
        index.insert("Bob", 2);
        index.insert("Alice", 3);

        const values = index.values();

        assertEqual(values.length, 2);
        assertContains(values, "Alice");
        assertContains(values, "Bob");
    });

    suite.test("获取所有主键", () => {
        const index = new SecondaryIndex({
            name: "name_idx",
            fieldName: "name",
        });

        index.insert("Alice", 1);
        index.insert("Alice", 2);
        index.insert("Bob", 3);

        const keys = index.keys();

        assertEqual(keys.length, 3);
    });

    suite.test("获取统计信息", () => {
        const index = new SecondaryIndex({
            name: "name_idx",
            fieldName: "name",
        });

        index.insert("Alice", 1);
        index.insert("Alice", 2);

        const stats = index.getStats();

        assertEqual(stats.name, "name_idx");
        assertEqual(stats.keyCount, 2);
        assertEqual(stats.entryCount, 1);
    });

    suite.test("验证索引一致性", () => {
        const index = new SecondaryIndex({
            name: "name_idx",
            fieldName: "name",
        });

        index.insert("Alice", 1);
        index.insert("Bob", 2);

        const result = index.validate();

        assertTrue(result.valid);
    });

    suite.test("清空索引", () => {
        const index = new SecondaryIndex({
            name: "name_idx",
            fieldName: "name",
        });

        index.insert("Alice", 1);
        index.clear();

        assertEqual(index.Size, 0);
        assertTrue(index.isEmpty());
    });

    return suite;
}

/**
 * 索引管理器测试套件
 */
function createIndexManagerTests(): TestSuite {
    interface TestDocument {
        id: number;
        name: string;
        age: number;
        email: string;
    }

    const suite = describe("IndexManager Tests");

    suite.test("创建索引管理器", () => {
        const config: CollectionIndexConfig<TestDocument> = {
            name: "users",
            primaryKey: "id",
            primaryKeyExtractor: (doc) => doc.id,
        };

        const manager = new IndexManager<TestDocument>(config);

        assertEqual(manager.CollectionName, "users");
        assertEqual(manager.PrimaryKeyField, "id");
        assertEqual(manager.Size, 0);
    });

    suite.test("创建带二级索引的管理器", () => {
        const config: CollectionIndexConfig<TestDocument> = {
            name: "users",
            primaryKey: "id",
            primaryKeyExtractor: (doc) => doc.id,
            secondaryIndexes: [
                {
                    name: "name_idx",
                    fieldName: "name",
                    valueExtractor: (doc) => doc.name,
                },
                {
                    name: "email_idx",
                    fieldName: "email",
                    unique: true,
                    valueExtractor: (doc) => doc.email,
                },
            ],
        };

        const manager = new IndexManager<TestDocument>(config);

        assertEqual(manager.getSecondaryIndexNames().length, 2);
        assertTrue(manager.hasSecondaryIndex("name_idx"));
        assertTrue(manager.hasSecondaryIndex("email_idx"));
    });

    suite.test("插入记录", () => {
        const config: CollectionIndexConfig<TestDocument> = {
            name: "users",
            primaryKey: "id",
            primaryKeyExtractor: (doc) => doc.id,
        };

        const manager = new IndexManager<TestDocument>(config);
        const doc: TestDocument = { id: 1, name: "Alice", age: 25, email: "alice@example.com" };

        const result = manager.insert(doc);

        assertTrue(result);
        assertEqual(manager.Size, 1);
    });

    suite.test("插入重复记录", () => {
        const config: CollectionIndexConfig<TestDocument> = {
            name: "users",
            primaryKey: "id",
            primaryKeyExtractor: (doc) => doc.id,
        };

        const manager = new IndexManager<TestDocument>(config);
        const doc: TestDocument = { id: 1, name: "Alice", age: 25, email: "alice@example.com" };

        manager.insert(doc);
        const result = manager.insert(doc);

        assertFalse(result);
    });

    suite.test("通过主键查找", () => {
        const config: CollectionIndexConfig<TestDocument> = {
            name: "users",
            primaryKey: "id",
            primaryKeyExtractor: (doc) => doc.id,
        };

        const manager = new IndexManager<TestDocument>(config);
        const doc: TestDocument = { id: 1, name: "Alice", age: 25, email: "alice@example.com" };

        manager.insert(doc);
        const found = manager.findByPrimaryKey(1);

        assertNotNull(found);
        assertEqual(found!.name, "Alice");
    });

    suite.test("通过二级索引查找", () => {
        const config: CollectionIndexConfig<TestDocument> = {
            name: "users",
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

        const manager = new IndexManager<TestDocument>(config);

        manager.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com" });
        manager.insert({ id: 2, name: "Alice", age: 30, email: "alice2@example.com" });
        manager.insert({ id: 3, name: "Bob", age: 28, email: "bob@example.com" });

        const results = manager.findBySecondaryIndex("name_idx", "Alice");

        assertEqual(results.length, 2);
    });

    suite.test("更新记录", () => {
        const config: CollectionIndexConfig<TestDocument> = {
            name: "users",
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

        const manager = new IndexManager<TestDocument>(config);

        const oldDoc: TestDocument = { id: 1, name: "Alice", age: 25, email: "alice@example.com" };
        manager.insert(oldDoc);

        const newDoc: TestDocument = { id: 1, name: "Alice Smith", age: 26, email: "alice@example.com" };
        const result = manager.update(oldDoc, newDoc);

        assertTrue(result);
        assertEqual(manager.findByPrimaryKey(1)!.name, "Alice Smith");
    });

    suite.test("删除记录", () => {
        const config: CollectionIndexConfig<TestDocument> = {
            name: "users",
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

        const manager = new IndexManager<TestDocument>(config);

        const doc: TestDocument = { id: 1, name: "Alice", age: 25, email: "alice@example.com" };
        manager.insert(doc);

        const deleted = manager.delete(doc);

        assertNotNull(deleted);
        assertEqual(manager.Size, 0);
    });

    suite.test("通过主键删除", () => {
        const config: CollectionIndexConfig<TestDocument> = {
            name: "users",
            primaryKey: "id",
            primaryKeyExtractor: (doc) => doc.id,
        };

        const manager = new IndexManager<TestDocument>(config);

        manager.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com" });
        const deleted = manager.deleteByKey(1);

        assertNotNull(deleted);
        assertEqual(manager.Size, 0);
    });

    suite.test("主键范围查询", () => {
        const config: CollectionIndexConfig<TestDocument> = {
            name: "users",
            primaryKey: "id",
            primaryKeyExtractor: (doc) => doc.id,
        };

        const manager = new IndexManager<TestDocument>(config);

        for (let i = 1; i <= 5; i++) {
            manager.insert({ id: i, name: `User${i}`, age: 20 + i, email: `user${i}@example.com` });
        }

        const results = manager.rangeByPrimaryKey(2, 4);

        assertEqual(results.length, 3);
    });

    suite.test("二级索引范围查询", () => {
        const config: CollectionIndexConfig<TestDocument> = {
            name: "users",
            primaryKey: "id",
            primaryKeyExtractor: (doc) => doc.id,
            secondaryIndexes: [
                {
                    name: "age_idx",
                    fieldName: "age",
                    valueExtractor: (doc) => doc.age,
                },
            ],
        };

        const manager = new IndexManager<TestDocument>(config);

        for (let i = 1; i <= 5; i++) {
            manager.insert({ id: i, name: `User${i}`, age: 20 + i * 5, email: `user${i}@example.com` });
        }

        const results = manager.rangeBySecondaryIndex("age_idx", 25, 35);

        assertGreater(results.length, 0);
    });

    suite.test("获取所有记录", () => {
        const config: CollectionIndexConfig<TestDocument> = {
            name: "users",
            primaryKey: "id",
            primaryKeyExtractor: (doc) => doc.id,
        };

        const manager = new IndexManager<TestDocument>(config);

        manager.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com" });
        manager.insert({ id: 2, name: "Bob", age: 30, email: "bob@example.com" });

        const all = manager.getAll();

        assertEqual(all.length, 2);
    });

    suite.test("创建二级索引", () => {
        const config: CollectionIndexConfig<TestDocument> = {
            name: "users",
            primaryKey: "id",
            primaryKeyExtractor: (doc) => doc.id,
        };

        const manager = new IndexManager<TestDocument>(config);

        manager.createSecondaryIndex({
            name: "age_idx",
            fieldName: "age",
            valueExtractor: (doc) => doc.age,
        });

        assertTrue(manager.hasSecondaryIndex("age_idx"));
    });

    suite.test("删除二级索引", () => {
        const config: CollectionIndexConfig<TestDocument> = {
            name: "users",
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

        const manager = new IndexManager<TestDocument>(config);

        const result = manager.dropSecondaryIndex("name_idx");

        assertTrue(result);
        assertFalse(manager.hasSecondaryIndex("name_idx"));
    });

    suite.test("批量插入", () => {
        const config: CollectionIndexConfig<TestDocument> = {
            name: "users",
            primaryKey: "id",
            primaryKeyExtractor: (doc) => doc.id,
        };

        const manager = new IndexManager<TestDocument>(config);

        const docs: TestDocument[] = [
            { id: 1, name: "Alice", age: 25, email: "alice@example.com" },
            { id: 2, name: "Bob", age: 30, email: "bob@example.com" },
            { id: 3, name: "Charlie", age: 28, email: "charlie@example.com" },
        ];

        const result = manager.insertBatch(docs);

        assertEqual(result.success, 3);
        assertEqual(result.failed, 0);
    });

    suite.test("批量删除", () => {
        const config: CollectionIndexConfig<TestDocument> = {
            name: "users",
            primaryKey: "id",
            primaryKeyExtractor: (doc) => doc.id,
        };

        const manager = new IndexManager<TestDocument>(config);

        for (let i = 1; i <= 5; i++) {
            manager.insert({ id: i, name: `User${i}`, age: 20 + i, email: `user${i}@example.com` });
        }

        const result = manager.deleteBatch([1, 3, 5]);

        assertEqual(result.success, 3);
        assertEqual(result.failed, 0);
    });

    suite.test("验证索引一致性", () => {
        const config: CollectionIndexConfig<TestDocument> = {
            name: "users",
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

        const manager = new IndexManager<TestDocument>(config);

        manager.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com" });
        manager.insert({ id: 2, name: "Bob", age: 30, email: "bob@example.com" });

        const result = manager.validate();

        assertTrue(result.valid);
    });

    suite.test("重建索引", () => {
        const config: CollectionIndexConfig<TestDocument> = {
            name: "users",
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

        const manager = new IndexManager<TestDocument>(config);

        manager.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com" });
        manager.insert({ id: 2, name: "Bob", age: 30, email: "bob@example.com" });

        manager.rebuild([
            { id: 1, name: "Alice", age: 26, email: "alice@example.com" },
            { id: 2, name: "Bob", age: 31, email: "bob@example.com" },
            { id: 3, name: "Charlie", age: 28, email: "charlie@example.com" },
        ]);

        assertEqual(manager.Size, 3);
    });

    suite.test("获取内存使用量", () => {
        const config: CollectionIndexConfig<TestDocument> = {
            name: "users",
            primaryKey: "id",
            primaryKeyExtractor: (doc) => doc.id,
        };

        const manager = new IndexManager<TestDocument>(config);

        manager.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com" });

        const memory = manager.getMemoryUsage();

        assertGreater(memory, 0);
    });

    return suite;
}

/**
 * 导出所有索引引擎测试
 */
export function getIndexEngineTests(): TestSuite[] {
    return [
        createBPlusTreeTests(),
        createPrimaryIndexTests(),
        createSecondaryIndexTests(),
        createIndexManagerTests(),
    ];
}
